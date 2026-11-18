# ADR-0001：musicat-core 的非同步模型

- 狀態：已採納（2026-11-18）
- 範圍：`musicat-core`（`rust/core/src/runtime.rs`、`timing.rs`、`http.rs`、`testing.rs`）
- 相關：`docs/superpowers/specs/2026-11-06-cross-platform-rust-design.md` 第 13 節

## 背景

Swift 版的歌詞查詢（`LyricsRepository.swift`）倚賴 Swift Concurrency 的四個行為：

1. **逾時賽跑**：每個來源 5 秒上限，逾時視同失敗，而且不等待不理會取消的來源（`FetchRace`）。
2. **併發合併**：同一首歌的併發查詢共用一次執行（`flights`），每個來源最多被問一次。
3. **取消**：所有呼叫者都被取消時才放棄該次查詢；只有部分取消時查詢繼續。
4. **取消不入快取**：被放棄的查詢不得被快取成「查無」。

core 不得依賴任何 async runtime（tokio、async-std、smol…），也不得碰平台 API；外殼用 GPUI 的執行器，Windows 版之後可能用別的。因此 core 需要一個「能表達上面四件事、又不綁執行器」的模型。這是 core 最貴的決定：之後每個移植測試的寫法都取決於它。

## 定案

| 項目 | 決定 | 理由 |
| --- | --- | --- |
| 基本單位 | 一般的 `Future`（`BoxFuture<'a, T> = Pin<Box<dyn Future + Send + 'a>>`），時間來自注入的 `Clock`（`now()`、`sleep(d)`） | Swift 的 async 程式碼可逐行對照移植；測試換成 `FakeClock` 即可決定性地推進時間。 |
| 逾時 | `with_timeout(clock, limit, fut)` 以 `futures_util::future::select` 讓 `fut` 與 `clock.sleep(limit)` 賽跑，輸家被 drop | Rust 的 drop 就是取消，不需要被包的 future 配合，對應 Swift「不等不理會取消的來源」。同一輪都就緒時 `fut` 優先，快的來源不會被切掉。 |
| 併發合併 | `Coalescer<K, V>`：內部 future 包成 `futures_util::future::Shared`，表中只存 `WeakShared`；每個呼叫者拿到一個持有強參考的等待 future | 表不持有強參考，所以最後一個等待者被 drop 時內部 future 自然被 drop（取消），不需要額外的取消通道；內部 future 完成時自己移除表項，之後同 key 重新執行。 |
| 取消不入快取 | 快取寫入寫在被合併的 future **內部**、位於結果算完之後 | future 沒跑完就被 drop，寫入那一行根本不會執行，所以取消自然不會被快取成「查無」，不需要特別判斷。 |
| 背景工作 | `Spawner::spawn(BoxFuture<'static, ()>) -> TaskHandle`，`TaskHandle` drop 即取消，`detach()` 讓它跑完 | 與 GPUI 的 `Task` 語意一致，外殼只要把 GPUI 的 task move 進 `TaskHandle::new` 即可；沒有原生取消的執行器用 `runtime::abortable_task` 包一層。 |
| 同步通知 | `Notifier<T>`：`Mutex<Vec<(id, Arc<dyn Fn>)>>`，`Subscription` drop 即取消訂閱；通知時先複製出訂閱者快照、鎖外呼叫 | 回呼內可再訂閱、取消或通知而不死結。 |
| HTTP | `HttpClient::get(HttpRequest) -> BoxFuture<Result<HttpResponse, HttpError>>`；`HttpError` 只有 `Transport(String)` 與 `TooLarge` | 實作（ureq 等）留在外殼；回傳的 future drop 即放棄請求，與逾時、合併的取消語意一致。 |
| 依賴 | 只用 `futures-util`（`default-features = false, features = ["std"]`） | 只有組合子、沒有執行器或 runtime，通過 `check-core-purity.sh`。 |

### 細節與不變式

- `with_timeout` 的計時從第一次輪詢開始；`limit = Duration::MAX` 只是永遠不到期的 sleep，不會 panic（對應 Swift `testInfiniteTimeoutDoesNotTrap`）。
- `Coalescer::run` 在呼叫當下就登記（不必等輪詢），所以 `waiters(&key)` 立即反映呼叫者數。`make` 在鎖外呼叫；多執行緒同時搶同一個新 key 時，輸家的 `make()` 結果在未輪詢前就被丟棄。
- `Coalescer` 的表項以每次執行的 id 標記：完成時與最後一個等待者離開時都只移除「自己那一次」的表項，不會誤刪後來的新執行。
- 表中的 `WeakShared` 升級失敗（強參考都沒了）一律視同沒有進行中的執行。
- `Coalescer` 與 `Notifier` 在持鎖時不執行任何使用者程式碼：移出的表項、回呼與內部 future 都在鎖外 drop／呼叫。
- `Coalescer` 的 `K` 需要 `Send + 'static`、`V` 需要 `Sync`：等待 future 必須是 `Send`，而 `Shared` 只有在輸出為 `Send + Sync` 時才是 `Send`；歌詞相關的值都是純資料，滿足這兩個條件。

### 測試替身（`testing` 模組，feature `testing`；本 crate 的單元測試一律可用）

- `FakeClock`：`advance(d)` 推進時間並在鎖外喚醒所有到期的 sleep；`set_now(t)` 只改牆上時間讀數（測快取 TTL），不影響 sleep；sleep 每次輪詢都更新為最新的 waker；`pending_sleeps()` 可驗證輸家 sleep 已被 drop。
- `FakeHttp`：依完整 URL 排程回應、錯誤或永不回應（`Hang`），記錄每一次請求。
- `LocalSpawner`：決定性的單執行緒執行器，`spawn` 只排入，`run_until_stalled()` 把所有工作跑到停滯。語意同 `futures::executor::LocalPool::run_until_stalled`，但 `LocalPool` 是 `!Send`／`!Sync`，無法實作 `Spawner: Send + Sync`，所以改以 `futures_util::stream::FuturesUnordered` 加一個喚醒旗標自行驅動，也因此不需要 `futures-executor`。

## 被否決的替案

**以事件加計時驅動的狀態機**（core 只提供 `on_event` / `on_tick(now)`，所有等待都由外殼的計時器推進）。

- 優點：完全不需要 `Future`，取消就是丟掉狀態。
- 否決理由：Swift 的歌詞流程是「依序走訪來源、每個來源逾時賽跑、合併併發呼叫」的線性 async 程式碼，改寫成狀態機後每個移植測試都要手動推進每個狀態轉移，與 Swift 測試無法逐案對照，改寫與審查成本高；而本 ADR 的三個證明測試顯示 `Future` 加注入 `Clock` 已能決定性地表達同樣的行為。

## 證明測試

以下三個測試（`rust/core/src/timing.rs`）證明 Swift `LyricsRepositoryTests` 的三類關鍵測試（併發合併、取消不入快取、逾時不卡住）在本模型下寫得出來：

1. `with_timeout_does_not_wait_for_a_future_that_ignores_cancellation`
2. `coalescer_shares_one_run_between_concurrent_callers`
3. `coalescer_cancels_the_run_when_every_waiter_is_dropped_and_never_reports_a_result`

補充的不變式測試：`coalescer_forgets_a_finished_run_before_every_waiter_has_observed_it`、`coalescer_drops_the_cancelled_run_outside_its_lock`、`coalescer_does_not_join_a_run_whose_waiter_futures_were_dropped_unpolled`。
