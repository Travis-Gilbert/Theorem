use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct RoundScheduler {
    idle_interval: Duration,
    active_interval: Duration,
    last_activity: Option<Instant>,
    active_window: Duration,
}

impl RoundScheduler {
    pub fn new(idle_interval: Duration, active_interval: Duration) -> Self {
        Self {
            idle_interval,
            active_interval,
            last_activity: None,
            active_window: Duration::from_secs(30),
        }
    }

    pub fn note_activity(&mut self, now: Instant) {
        self.last_activity = Some(now);
    }

    pub fn current_interval(&self, now: Instant) -> Duration {
        match self.last_activity {
            Some(last) if now.duration_since(last) <= self.active_window => self.active_interval,
            _ => self.idle_interval,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_uses_active_interval_after_activity() {
        let now = Instant::now();
        let mut scheduler = RoundScheduler::new(Duration::from_secs(30), Duration::from_secs(5));
        assert_eq!(scheduler.current_interval(now), Duration::from_secs(30));
        scheduler.note_activity(now);
        assert_eq!(
            scheduler.current_interval(now + Duration::from_secs(1)),
            Duration::from_secs(5)
        );
        assert_eq!(
            scheduler.current_interval(now + Duration::from_secs(31)),
            Duration::from_secs(30)
        );
    }
}
