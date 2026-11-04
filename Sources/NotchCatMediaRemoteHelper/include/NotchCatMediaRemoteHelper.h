#ifndef NOTCHCAT_MEDIA_REMOTE_HELPER_H
#define NOTCHCAT_MEDIA_REMOTE_HELPER_H

/// Entry point called by `/usr/bin/perl` as an XSUB (hence the `interp`, `cv`
/// signature; both are ignored). Never returns: it reports now-playing changes
/// on stdout as JSON lines and exits the process when stdin reaches EOF.
void notchcat_helper_main(void *interp, void *cv);

#endif
