use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// 全局令牌桶限速器：允许"欠账"，调用方按欠账时长休眠
pub struct Limiter {
    /// 字节/秒，0 表示不限速
    rate: AtomicU64,
    state: Mutex<(f64, Instant)>,
}

impl Limiter {
    pub fn new(rate: u64) -> Self {
        Self {
            rate: AtomicU64::new(rate),
            state: Mutex::new((rate as f64, Instant::now())),
        }
    }

    pub fn set_rate(&self, rate: u64) {
        if self.rate.swap(rate, Ordering::Relaxed) == rate {
            return;
        }
        *self.state.lock() = (rate as f64, Instant::now());
    }

    /// 预留 n 个字节，返回需要等待的时长
    pub fn reserve(&self, n: usize) -> Duration {
        let rate = self.rate.load(Ordering::Relaxed);
        if rate == 0 {
            return Duration::ZERO;
        }
        let mut st = self.state.lock();
        let now = Instant::now();
        let elapsed = now.duration_since(st.1).as_secs_f64();
        // 突发上限为 1 秒的配额
        st.0 = (st.0 + elapsed * rate as f64).min(rate as f64);
        st.1 = now;
        st.0 -= n as f64;
        if st.0 < 0.0 {
            Duration::from_secs_f64(-st.0 / rate as f64)
        } else {
            Duration::ZERO
        }
    }
}
