/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

use std::time::{ Duration, Instant };

/// Implements a simple non-linear backoff strategy
pub struct Backoff {
    restart_count: u64,
    restart_threshold: Duration,
    start_time: Option<Instant>,
    backoff: u64,
}

impl Backoff {

    pub fn new(restart_threshold: Duration) -> Self {
        Backoff {
            restart_count: 0,
            restart_threshold: restart_threshold,
            start_time: None,
            backoff: 1,
        }
    }

    pub fn from_secs(restart_threshold_secs: u64) -> Self {
        Backoff::new(Duration::from_secs(restart_threshold_secs))
    }

    pub fn next_backoff(&mut self) -> Duration {
        let end_time = Instant::now();

        let duration_to_backoff =
            if let Some(start_time) = self.start_time {
                if (end_time - start_time) < self.restart_threshold {
                    self.backoff += 1;

                    // non-linear back off
                    Duration::from_secs((self.backoff * self.backoff) >> 1)
                } else {
                    self.backoff = 1;
                    Duration::from_secs(0)
                }
            } else {
                Duration::from_secs(0)
            };

        self.restart_count += 1;
        self.start_time = Some(Instant::now());

        duration_to_backoff
    }

    pub fn get_restart_count(&self) -> u64 {
        self.restart_count
    }
}

#[cfg(test)]
mod test {
    use super::Backoff;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_backoff_immediate_if_failed_after_threshold() {

        let mut backoff = Backoff::from_secs(2);
        assert_eq!(backoff.next_backoff().as_secs(), 0);

        // Simulate process running
        thread::sleep(Duration::new(4, 0));

        assert_eq!(backoff.next_backoff().as_secs(), 0);
    }

    #[test]
    fn test_backoff_wait_if_failed_before_threshold() {
        let mut backoff = Backoff::from_secs(1);
        assert_eq!(backoff.next_backoff().as_secs(), 0);

        assert_eq!(backoff.next_backoff().as_secs(), 2);
        assert_eq!(backoff.next_backoff().as_secs(), 4);
        assert_eq!(backoff.next_backoff().as_secs(), 8);
        assert_eq!(backoff.next_backoff().as_secs(), 12);
        assert_eq!(backoff.next_backoff().as_secs(), 18);
        assert_eq!(backoff.next_backoff().as_secs(), 24);
    }

    #[test]
    fn test_backoff_reset_if_running_for_more_than_threshold() {
        let mut backoff = Backoff::from_secs(1);
        assert_eq!(backoff.next_backoff().as_secs(), 0);
        assert_eq!(backoff.next_backoff().as_secs(), 2);
        assert_eq!(backoff.next_backoff().as_secs(), 4);
        assert_eq!(backoff.next_backoff().as_secs(), 8);

        // Simulate process running
        thread::sleep(Duration::new(3, 0));

        assert_eq!(backoff.next_backoff().as_secs(), 0);
    }
}
