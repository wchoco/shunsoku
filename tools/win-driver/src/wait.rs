//! 状態を待つ。決め打ちの時間ではなく、条件が満たされるまで短い間隔で見直し、時間切れなら失敗にする。

use std::time::{Duration, Instant};

/// 見直す間隔。
pub const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// 時間を進めるもの。テストでは偽の時計を使う。
pub trait Clock {
    fn now(&mut self) -> Duration;
    fn sleep(&mut self, d: Duration);
}

/// 本物の時計。
pub struct RealClock(Instant);

impl RealClock {
    pub fn new() -> RealClock {
        RealClock(Instant::now())
    }
}

impl Clock for RealClock {
    fn now(&mut self) -> Duration {
        self.0.elapsed()
    }
    fn sleep(&mut self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// 時間切れ。
#[derive(Debug, PartialEq)]
pub struct TimedOut;

/// `probe` が Some を返すまで待つ。
pub fn wait_for<T>(
    clock: &mut impl Clock,
    timeout: Duration,
    mut probe: impl FnMut() -> Option<T>,
) -> Result<T, TimedOut> {
    let deadline = clock.now() + timeout;
    loop {
        if let Some(v) = probe() {
            return Ok(v);
        }
        let now = clock.now();
        if now >= deadline {
            return Err(TimedOut);
        }
        clock.sleep(POLL_INTERVAL.min(deadline - now));
    }
}

/// ログ `content` の、先頭の `skip` 行より後で `needle` を含む最初の行。書きかけの最後の行（改行がない）は見ない。
pub fn find_line<'a>(content: &'a str, skip: usize, needle: &str) -> Option<&'a str> {
    complete_lines(content)
        .skip(skip)
        .find(|line| line.contains(needle))
}

/// 改行で終わった行。行末の `\r` は落とす。
pub fn complete_lines(content: &str) -> impl Iterator<Item = &str> {
    let complete = content.rfind('\n').map_or("", |i| &content[..=i]);
    complete.lines()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_line_after_the_skipped_ones() {
        let log = "a saved 1\r\nb other\r\nc saved 2\r\n";
        assert_eq!(find_line(log, 0, "saved"), Some("a saved 1"));
        assert_eq!(find_line(log, 1, "saved"), Some("c saved 2"));
        assert_eq!(find_line(log, 3, "saved"), None);
    }

    #[test]
    fn ignores_a_line_still_being_written() {
        assert_eq!(find_line("x\nsav", 0, "sav"), None);
    }

    /// 眠った分だけ進む時計。
    #[derive(Default)]
    struct FakeClock {
        t: Duration,
    }

    impl Clock for FakeClock {
        fn now(&mut self) -> Duration {
            self.t
        }
        fn sleep(&mut self, d: Duration) {
            self.t += d;
        }
    }

    #[test]
    fn returns_as_soon_as_the_condition_holds() {
        let mut clock = FakeClock::default();
        let mut calls = 0;
        let got = wait_for(&mut clock, Duration::from_secs(5), || {
            calls += 1;
            (calls == 3).then_some("ready")
        });
        assert_eq!(got, Ok("ready"));
        assert_eq!(clock.t, POLL_INTERVAL * 2);
    }

    #[test]
    fn times_out_after_the_given_duration() {
        let mut clock = FakeClock::default();
        let got: Result<(), _> = wait_for(&mut clock, Duration::from_millis(1000), || None);
        assert_eq!(got, Err(TimedOut));
        assert!(clock.t >= Duration::from_millis(1000));
        assert!(clock.t < Duration::from_millis(1000) + POLL_INTERVAL);
    }

    #[test]
    fn checks_once_more_at_the_deadline() {
        // 時間切れの直前に満たされた条件を取りこぼさない
        // 0, 50, 100 ms に見て、最後は 120 ms ちょうどに見る
        let mut clock = FakeClock::default();
        let mut calls = 0;
        let got = wait_for(&mut clock, Duration::from_millis(120), || {
            calls += 1;
            (calls == 4).then_some(())
        });
        assert_eq!(got, Ok(()));
        assert_eq!(clock.t, Duration::from_millis(120));
    }

    #[test]
    fn zero_timeout_still_checks_once() {
        let mut clock = FakeClock::default();
        assert_eq!(wait_for(&mut clock, Duration::ZERO, || Some(1)), Ok(1));
    }
}
