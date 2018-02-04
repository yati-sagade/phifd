use std::collections::VecDeque;
use std::time::{Instant, Duration};

use proto::msg::Member;
use statrs::distribution::{Normal, Univariate};
use statrs::statistics::{Mean, Variance};
use statrs::statistics::{Statistics};

/// This type is used to identify a member uniquely using its IPv4 number and
/// port.
pub type MemberID = (u32, u16);

#[derive(Clone, Debug)]
pub struct InterArrivalWindow {
    distribution: Option<Normal>,
    window: VecDeque<Duration>,
    last_arrival_at: Option<Instant>,
    size: usize,
    running_stats: Option<(f64, f64)>, // Running (mean, variance)
}

impl InterArrivalWindow {

    pub fn of_size(size: usize) -> InterArrivalWindow {
        assert!(size >= 3);
        InterArrivalWindow {
            distribution: None,
            window: VecDeque::with_capacity(size),
            size: size,
            last_arrival_at: None,
            running_stats: None,
        }
    }

    pub fn size(&self) -> usize { self.size }

    /// Update estimates of the mean and variance of inter-arrival times by
    /// adding an observation `value` to the window.
    pub fn update(&mut self, value: Duration) {

        let duration_secs = value.as_secs() as f64
                          + value.subsec_nanos() as f64 * 1e-9;
        self.running_stats = self.running_stats.map(|(old_mean, old_var)| {
            let new_mean = 0.9 * old_mean + 0.1 * duration_secs;
            let new_var = 0.9 * old_var +
                0.1 * (duration_secs - old_mean) * (duration_secs - new_mean);
            (new_mean, new_var)
        }).or(Some((duration_secs, 0f64)));

        if self.window.len() == self.size {
            self.window.pop_front().expect("zero window size encountered");
        }
        self.window.push_back(value);
        if self.window.len() >= 2 {

            let raw_secs = self.window.iter().map(|dur| {
                dur.as_secs() as f64 + dur.subsec_nanos() as f64 * 1e-9
            }).collect::<Vec<f64>>();

            let mu = Statistics::mean(&raw_secs);
            let sigma = Statistics::variance(&raw_secs).sqrt();
            let dist = Normal::new(mu, sigma)
                    .expect("failed to create a normal distribution object!");
            let (m, v) = self.running_stats.unwrap();
            println!("after update, regular mean: {}, running: {}, regular var: {}, running: {}",
                     Mean::mean(&dist), m, Variance::variance(&dist), v);
            self.distribution = Some(dist);
        }

    }   

    /// Trigger an update of the mean and variance estimates of the
    /// inter-arrival times as though a message arrived at the instant given
    /// by `arrival_time`.
    pub fn tick(&mut self, arrival_time: Instant) {
        if let Some(last_arrival) = self.last_arrival_at {
            self.update(arrival_time.duration_since(last_arrival));
        }
        self.last_arrival_at = Some(arrival_time);
    }

    pub fn phi(&self, at: Instant) -> Option<f64> {
        if let Some(last_arrival) = self.last_arrival_at {
            if last_arrival > at {
                None
            } else {
                self.distribution.map(|dist| {
                    let dur = at.duration_since(last_arrival);
                    let dur_secs = dur.as_secs() as f64 +
                        dur.subsec_nanos() as f64 * 1e-9;
                    -(1.0 - dist.cdf(dur_secs)).log10()
                })
            }
        } else {
            None
        }
    }

    #[cfg(test)]
    pub fn mean(&self) -> Option<f64> {
        self.distribution.as_ref().map(|d| Mean::mean(d))
    }

    #[cfg(test)]
    pub fn variance(&self) -> Option<f64> {
        self.distribution.as_ref().map(|d| Variance::variance(d))
    }
}

/// This stores this process' knowledge about a given member at any given time.
#[derive(Clone, Debug)]
pub struct MemberState {

    /// Information about the member that we need to gossip with other peers.
    member: Member,

    /// The last time when this member's state was updated.
    timestamp: Instant,

    window_size: usize,

    /// Book keeping for estimating the distribution of inter-arrival times of
    /// pings from this node.
    inter_arrival_window: Option<InterArrivalWindow>,
}

impl MemberState {
    pub fn from_member(member: Member, window_size: usize) -> MemberState {
        MemberState {
            member: member,
            timestamp: Instant::now(),
            window_size: window_size,
            inter_arrival_window: None,
        }
    }

    pub fn merge(&mut self, _suspicion: f64, heartbeat: u64) {
        if self.member.get_heartbeat() < heartbeat {
            self.member.set_heartbeat(heartbeat);

            self.timestamp = Instant::now();

            let wnd_sz = self.window_size;
            self.inter_arrival_window
                .get_or_insert_with(|| InterArrivalWindow::of_size(wnd_sz))
                .tick(Instant::now());
        }
    }

    pub fn phi(&self, at: Instant) -> Option<f64> {
        self.inter_arrival_window.as_ref().and_then(|iaw| iaw.phi(at))
    }

    pub fn get_id(&self) -> MemberID {
        (self.member.get_ip(), self.member.get_port() as u16)
    }

    pub fn get_member_ref<'a>(&'a self) -> &'a Member {
        &self.member
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }

    #[test]
    fn test_inter_arrival_interval_window_ctor() {
        let interval = InterArrivalWindow::of_size(3);
        assert!(interval.mean().is_none());
        assert!(interval.variance().is_none());
        assert_eq!(interval.size(), 3);
    }

    #[test]
    fn test_inter_arrival_interval_window_update() {
        let mut interval = InterArrivalWindow::of_size(3);
        
        interval.update(Duration::from_secs(3));
        assert!(interval.mean().is_none());
        assert!(interval.variance().is_none());
        assert_eq!(interval.size(), 3);

        interval.update(Duration::from_secs(4));
        assert!((interval.mean().unwrap() - 3.5f64).abs() <= 1e-6);
        assert!((interval.variance().unwrap() - 0.5f64).abs() <= 1e-6);
        assert_eq!(interval.size(), 3);

        interval.update(Duration::from_secs(100));
        assert!((interval.mean().unwrap() - 35.6666666f64).abs() <= 10e-6);
        assert!((interval.variance().unwrap() - 3104.33333f64).abs() <= 10e-6);
        assert_eq!(interval.size(), 3);
    }
}
