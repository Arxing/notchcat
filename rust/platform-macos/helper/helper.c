// MusiCat MediaRemote helper.
//
// This is a copy of the Swift version's Sources/NotchCatMediaRemoteHelper/helper.c
// with only the entry symbol renamed (notchcat_helper_main -> musicat_helper_main)
// and the NotchCat wording in this header changed to MusiCat. The wire format,
// timeouts, exit codes and restart logic are unchanged.
//
// Built by platform-macos/build.rs as libmusicat_mediaremote_helper.dylib and
// loaded ONLY inside an Apple-signed `/usr/bin/perl` through DynaLoader (see the
// perl launcher).
// A third-party-signed process gets an empty dictionary from
// MRMediaRemoteGetNowPlayingInfo; the Apple-signed host gets the real data.
//
// Wire format (one JSON object per line on stdout, parsed by HelperMessageParser):
//
//     {"none":true}
//     {"title":String,"artist":String?,"album":String?,"duration":Number?,
//      "elapsed":Number,"timestamp":Number,"rate":Number,
//      "artworkId":String?,"artworkData":String?}
//
// Absent values are omitted, never written as null. `timestamp` is Unix epoch
// seconds. `artworkData` is base64 and only present when MediaRemote attached
// the bytes to this reply and they are at most kMaxArtworkBytes. A line is
// written only when title, artist, rate, elapsed, timestamp, artworkId or
// "artwork bytes attached" changed since the previous line. The process exits when stdin reaches EOF, so it dies with the
// parent that holds the pipe.
//
// Exit codes: 0 stdin reached EOF; 2 setup failed (framework or symbol
// missing, watcher thread); 3 out of memory; 4 MediaRemote stopped replying
// (kMaxConsecutiveTimeouts requests in a row timed out; any reply resets the
// count), so the supervisor restarts the helper instead of leaving the app on
// a stale snapshot.
//
// Signing: the linker output is already ad-hoc signed on arm64
// (`codesign -dv` reports `Signature=adhoc`, `flags=0x20002(adhoc,linker-signed)`),
// which is all perl needs. No post-build codesign step is required.
//
// Observed on macOS 15.7 (2026-11-04):
// - Playing: a track line with rate 1 and the real title/artist/album/duration.
// - Paused: one passive sample, taken while the player happened to be paused,
//   returned the full dictionary with PlaybackRate 0, i.e. a track line with
//   "rate":0, not {"none":true}. A deliberate pause/resume transition was NOT
//   exercised (verification must not touch the user's playback).
// UNVERIFIED (to be checked on hardware by the user):
// - The output shape across an actual pause transition.
// - Whether seeking emits a new line (expected: ElapsedTime/Timestamp change).

#include "helper.h"

#include <CoreFoundation/CoreFoundation.h>
#include <dispatch/dispatch.h>
#include <dlfcn.h>
#include <math.h>
#include <pthread.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <errno.h>

typedef void (^NowPlayingInfoHandler)(CFDictionaryRef info);
typedef void (*GetNowPlayingInfoFn)(dispatch_queue_t queue, NowPlayingInfoHandler handler);

static const char *kFrameworkPath = "/System/Library/PrivateFrameworks/MediaRemote.framework/MediaRemote";
static const int64_t kReplyTimeoutSeconds = 3;
static const unsigned int kPollIntervalSeconds = 1;
// Consecutive requests without any reply before the process gives up (exit 4).
static const int kMaxConsecutiveTimeouts = 10;
/// Artwork larger than this is not attached (artworkId still is), so the line
/// stays under the Swift reader's 4 MiB line cap: 3,000,000 bytes encode to
/// 4,000,000 base64 characters, leaving ~190 KB for the other fields.
static const CFIndex kMaxArtworkBytes = 3000000;

// MediaRemote autoreleases objects on the calling thread. perl's main thread has
// no autorelease pool of its own, so each poll pushes and pops one; without it
// every poll leaks a few hundred bytes for the life of the process. libobjc is
// already a load command of the built dylib (see `otool -L`).
extern void *objc_autoreleasePoolPush(void);
extern void objc_autoreleasePoolPop(void *pool);

// MARK: - Growable output buffer

typedef struct {
    char *bytes;
    size_t length;
    size_t capacity;
} Buffer;

static void buffer_reserve(Buffer *buffer, size_t extra) {
    if (buffer->length + extra <= buffer->capacity) return;
    size_t capacity = buffer->capacity ? buffer->capacity : 256;
    while (capacity < buffer->length + extra) capacity *= 2;
    char *bytes = realloc(buffer->bytes, capacity);
    if (!bytes) _exit(3);
    buffer->bytes = bytes;
    buffer->capacity = capacity;
}

static void buffer_append(Buffer *buffer, const char *bytes, size_t length) {
    buffer_reserve(buffer, length);
    memcpy(buffer->bytes + buffer->length, bytes, length);
    buffer->length += length;
}

static void buffer_append_cstring(Buffer *buffer, const char *string) {
    buffer_append(buffer, string, strlen(string));
}

/// Appends `string` (UTF-8) as a quoted JSON string. Quotes, backslashes and
/// control characters are escaped; every other byte, including multi-byte
/// UTF-8 sequences for non-BMP characters, passes through unchanged.
static void buffer_append_json_string(Buffer *buffer, const char *string) {
    buffer_append(buffer, "\"", 1);
    for (const unsigned char *p = (const unsigned char *)string; *p; p++) {
        unsigned char c = *p;
        switch (c) {
            case '"': buffer_append(buffer, "\\\"", 2); break;
            case '\\': buffer_append(buffer, "\\\\", 2); break;
            case '\n': buffer_append(buffer, "\\n", 2); break;
            case '\r': buffer_append(buffer, "\\r", 2); break;
            case '\t': buffer_append(buffer, "\\t", 2); break;
            case '\b': buffer_append(buffer, "\\b", 2); break;
            case '\f': buffer_append(buffer, "\\f", 2); break;
            default:
                if (c < 0x20) {
                    char escaped[7];
                    snprintf(escaped, sizeof escaped, "\\u%04x", c);
                    buffer_append(buffer, escaped, 6);
                } else {
                    buffer_append(buffer, (const char *)p, 1);
                }
        }
    }
    buffer_append(buffer, "\"", 1);
}

static void buffer_append_number(Buffer *buffer, double value) {
    char text[32];
    int length = snprintf(text, sizeof text, "%.17g", value);
    buffer_append(buffer, text, (size_t)length);
}

static void buffer_append_base64(Buffer *buffer, const UInt8 *bytes, CFIndex length) {
    static const char alphabet[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    buffer_reserve(buffer, (size_t)((length + 2) / 3) * 4);
    for (CFIndex i = 0; i < length; i += 3) {
        uint32_t chunk = (uint32_t)bytes[i] << 16;
        if (i + 1 < length) chunk |= (uint32_t)bytes[i + 1] << 8;
        if (i + 2 < length) chunk |= bytes[i + 2];
        char quad[4] = {
            alphabet[(chunk >> 18) & 0x3F],
            alphabet[(chunk >> 12) & 0x3F],
            i + 1 < length ? alphabet[(chunk >> 6) & 0x3F] : '=',
            i + 2 < length ? alphabet[chunk & 0x3F] : '=',
        };
        buffer_append(buffer, quad, 4);
    }
}

// MARK: - Reading the info dictionary

static CFTypeRef info_value(CFDictionaryRef info, const char *key) {
    CFStringRef cfKey = CFStringCreateWithCString(kCFAllocatorDefault, key, kCFStringEncodingUTF8);
    CFTypeRef value = CFDictionaryGetValue(info, cfKey);
    CFRelease(cfKey);
    return value;
}

/// Returns a malloc'd UTF-8 copy of a non-empty string value, or NULL.
static char *info_string(CFDictionaryRef info, const char *key) {
    CFTypeRef value = info_value(info, key);
    if (!value || CFGetTypeID(value) != CFStringGetTypeID()) return NULL;
    CFStringRef string = (CFStringRef)value;
    if (CFStringGetLength(string) == 0) return NULL;
    CFIndex size = CFStringGetMaximumSizeForEncoding(CFStringGetLength(string), kCFStringEncodingUTF8) + 1;
    char *bytes = malloc((size_t)size);
    if (!bytes) _exit(3);
    if (!CFStringGetCString(string, bytes, size, kCFStringEncodingUTF8)) {
        free(bytes);
        return NULL;
    }
    return bytes;
}

/// Reads a finite number; `false` when absent, of another type, or non-finite.
static bool info_number(CFDictionaryRef info, const char *key, double *out) {
    CFTypeRef value = info_value(info, key);
    if (!value || CFGetTypeID(value) != CFNumberGetTypeID()) return false;
    double number = 0;
    if (!CFNumberGetValue((CFNumberRef)value, kCFNumberDoubleType, &number) || !isfinite(number)) return false;
    *out = number;
    return true;
}

/// Reads a date as Unix epoch seconds.
static bool info_epoch(CFDictionaryRef info, const char *key, double *out) {
    CFTypeRef value = info_value(info, key);
    if (!value || CFGetTypeID(value) != CFDateGetTypeID()) return false;
    double epoch = CFDateGetAbsoluteTime((CFDateRef)value) + kCFAbsoluteTimeIntervalSince1970;
    if (!isfinite(epoch)) return false;
    *out = epoch;
    return true;
}

// MARK: - Change detection

/// The fields whose change triggers a new line.
typedef struct {
    bool none;
    char *title;
    char *artist;
    bool hasRate, hasElapsed, hasTimestamp;
    double rate, elapsed, timestamp;
    char *artworkId;
    bool hasArtworkData;
} State;

static bool strings_equal(const char *a, const char *b) {
    if (!a || !b) return a == b;
    return strcmp(a, b) == 0;
}

static bool optional_numbers_equal(bool hasA, double a, bool hasB, double b) {
    return hasA == hasB && (!hasA || a == b);
}

static bool states_equal(const State *a, const State *b) {
    if (a->none || b->none) return a->none == b->none;
    return strings_equal(a->title, b->title)
        && strings_equal(a->artist, b->artist)
        && optional_numbers_equal(a->hasRate, a->rate, b->hasRate, b->rate)
        && optional_numbers_equal(a->hasElapsed, a->elapsed, b->hasElapsed, b->elapsed)
        && optional_numbers_equal(a->hasTimestamp, a->timestamp, b->hasTimestamp, b->timestamp)
        && strings_equal(a->artworkId, b->artworkId)
        && a->hasArtworkData == b->hasArtworkData;
}

static void state_free(State *state) {
    free(state->title);
    free(state->artist);
    free(state->artworkId);
    memset(state, 0, sizeof *state);
}

// Only touched on `replyQueue`, which is serial.
static State lastState;
static bool hasLastState = false;

static void write_line(Buffer *line) {
    buffer_append(line, "\n", 1);
    fwrite(line->bytes, 1, line->length, stdout);
    fflush(stdout);
}

static void handle_info(CFDictionaryRef info) {
    State state = {0};
    char *album = NULL;
    double duration = 0;
    bool hasDuration = false;
    CFDataRef artworkData = NULL;

    if (info) state.title = info_string(info, "kMRMediaRemoteNowPlayingInfoTitle");
    if (!state.title) {
        state.none = true;
    } else {
        state.artist = info_string(info, "kMRMediaRemoteNowPlayingInfoArtist");
        album = info_string(info, "kMRMediaRemoteNowPlayingInfoAlbum");
        hasDuration = info_number(info, "kMRMediaRemoteNowPlayingInfoDuration", &duration);
        state.hasElapsed = info_number(info, "kMRMediaRemoteNowPlayingInfoElapsedTime", &state.elapsed);
        state.hasTimestamp = info_epoch(info, "kMRMediaRemoteNowPlayingInfoTimestamp", &state.timestamp);
        state.hasRate = info_number(info, "kMRMediaRemoteNowPlayingInfoPlaybackRate", &state.rate);
        state.artworkId = info_string(info, "kMRMediaRemoteNowPlayingInfoArtworkIdentifier");
        CFTypeRef data = info_value(info, "kMRMediaRemoteNowPlayingInfoArtworkData");
        if (data && CFGetTypeID(data) == CFDataGetTypeID()
            && CFDataGetLength((CFDataRef)data) > 0 && CFDataGetLength((CFDataRef)data) <= kMaxArtworkBytes) {
            artworkData = (CFDataRef)data;
            state.hasArtworkData = true;
        }
    }

    if (hasLastState && states_equal(&state, &lastState)) {
        state_free(&state);
        free(album);
        return;
    }

    Buffer line = {0};
    if (state.none) {
        buffer_append_cstring(&line, "{\"none\":true}");
    } else {
        buffer_append_cstring(&line, "{\"title\":");
        buffer_append_json_string(&line, state.title);
        if (state.artist) {
            buffer_append_cstring(&line, ",\"artist\":");
            buffer_append_json_string(&line, state.artist);
        }
        if (album) {
            buffer_append_cstring(&line, ",\"album\":");
            buffer_append_json_string(&line, album);
        }
        if (hasDuration) {
            buffer_append_cstring(&line, ",\"duration\":");
            buffer_append_number(&line, duration);
        }
        if (state.hasElapsed) {
            buffer_append_cstring(&line, ",\"elapsed\":");
            buffer_append_number(&line, state.elapsed);
        }
        if (state.hasTimestamp) {
            buffer_append_cstring(&line, ",\"timestamp\":");
            buffer_append_number(&line, state.timestamp);
        }
        if (state.hasRate) {
            buffer_append_cstring(&line, ",\"rate\":");
            buffer_append_number(&line, state.rate);
        }
        if (state.artworkId) {
            buffer_append_cstring(&line, ",\"artworkId\":");
            buffer_append_json_string(&line, state.artworkId);
        }
        if (artworkData) {
            buffer_append_cstring(&line, ",\"artworkData\":\"");
            buffer_append_base64(&line, CFDataGetBytePtr(artworkData), CFDataGetLength(artworkData));
            buffer_append(&line, "\"", 1);
        }
        buffer_append(&line, "}", 1);
    }
    write_line(&line);
    free(line.bytes);
    free(album);

    if (hasLastState) state_free(&lastState);
    lastState = state;
    hasLastState = true;
}

// MARK: - Lifetime

/// Exits the whole host process once stdin hits EOF (the parent closed its end
/// of the pipe or died).
static void *watch_stdin(void *unused) {
    (void)unused;
    char scratch[256];
    for (;;) {
        ssize_t count = read(STDIN_FILENO, scratch, sizeof scratch);
        if (count == 0 || (count < 0 && errno != EINTR)) _exit(0);
    }
    return NULL;
}

void musicat_helper_main(void *interp, void *cv) {
    (void)interp;
    (void)cv;

    pthread_t watcher;
    if (pthread_create(&watcher, NULL, watch_stdin, NULL) != 0) {
        fprintf(stderr, "notchcat-helper: cannot watch stdin\n");
        _exit(2);
    }
    pthread_detach(watcher);

    void *framework = dlopen(kFrameworkPath, RTLD_NOW);
    if (!framework) {
        fprintf(stderr, "notchcat-helper: dlopen failed: %s\n", dlerror());
        _exit(2);
    }
    GetNowPlayingInfoFn getNowPlayingInfo = (GetNowPlayingInfoFn)dlsym(framework, "MRMediaRemoteGetNowPlayingInfo");
    if (!getNowPlayingInfo) {
        fprintf(stderr, "notchcat-helper: MRMediaRemoteGetNowPlayingInfo not found\n");
        _exit(2);
    }

    // Replies are handled on one serial queue, so a reply arriving after its
    // wait timed out still diffs against consistent state.
    dispatch_queue_t replyQueue = dispatch_queue_create("notchcat.helper.reply", DISPATCH_QUEUE_SERIAL);
    dispatch_semaphore_t replied = dispatch_semaphore_create(0);
    int consecutiveTimeouts = 0;

    for (;;) {
        void *pool = objc_autoreleasePoolPush();
        getNowPlayingInfo(replyQueue, ^(CFDictionaryRef info) {
            handle_info(info);
            dispatch_semaphore_signal(replied);
        });
        long timedOut = dispatch_semaphore_wait(replied, dispatch_time(DISPATCH_TIME_NOW, kReplyTimeoutSeconds * NSEC_PER_SEC));
        if (timedOut) {
            if (++consecutiveTimeouts >= kMaxConsecutiveTimeouts) {
                fprintf(stderr, "notchcat-helper: MediaRemote stopped replying\n");
                _exit(4);
            }
        } else {
            consecutiveTimeouts = 0;
        }
        sleep(kPollIntervalSeconds);
        objc_autoreleasePoolPop(pool);
    }
}
