use std::collections::VecDeque;
use std::time::{Instant, Duration};

use proto::msg::Member;
use statrs::distribution::{Normal, Univariate};
use statrs::statistics::{Mean, Variance};
use statrs::statistics::Statistics;
use std::f64::consts::PI;

/// This type is used to identify a member uniquely using its IPv4 number and
/// port.
pub type MemberID = (u32, u16);


/// A normal distribution maintained using weighted averages.
#[derive(Clone, Debug)]
pub struct InterArrivalDistribution {
    mu: f64,
    sigma: f64,
    var: f64,
    alpha: f64,

    std_normal_variate: Normal,
}

impl InterArrivalDistribution {
    pub fn new(mu: f64, sigma: f64, alpha: f64) -> InterArrivalDistribution {
        InterArrivalDistribution {
            mu: mu,
            sigma: sigma,
            var: sigma * sigma,
            alpha: alpha,
            std_normal_variate: Normal::new(0f64, 1f64).unwrap(),
        }
    }

    pub fn mean(&self) -> f64 {
        self.mu
    }

    pub fn stddev(&self) -> f64 {
        self.sigma
    }

    pub fn variance(&self) -> f64 {
        self.var
    }

    pub fn update(&mut self, val: f64) {

        let new_mean = self.alpha * self.mean() + (1f64 - self.alpha) * val;

        let new_var = self.alpha * self.variance() +
            (1f64 - self.alpha) * (val - self.mean()) * (val - new_mean);

        println!("Update with {:0.4}, mean: {:0.4} -> {:0.4}, var: {:0.4}  -> {:0.4}",
                    val, self.mean(), new_mean, self.variance(), new_var);
        self.mu = new_mean;
        self.var = new_var;
        self.sigma = new_var.sqrt();
    }

    pub fn cdf(&self, val: f64) -> Option<f64> {
        if self.sigma == 0f64 {
            None
        } else {
            let z = (val - self.mean()) / self.stddev();
            let cdf = self.std_normal_variate.cdf(z);
            Some(cdf)
        }
    }
}

#[derive(Clone, Debug)]
pub struct InterArrivalWindow {
    distribution: Option<InterArrivalDistribution>,
    last_arrival_at: Option<Instant>,
    size: usize,
}

impl InterArrivalWindow {
    pub fn of_size(size: usize) -> InterArrivalWindow {
        assert!(size >= 3);
        InterArrivalWindow {
            distribution: None,
            size: size,
            last_arrival_at: None,
        }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// Update estimates of the mean and variance of inter-arrival times by
    /// adding an observation `value` to the window.
    pub fn update(&mut self, value: Duration) {

        let duration_secs = value.as_secs() as f64
                          + value.subsec_nanos() as f64 * 1e-9;

        if let Some(ref mut interdist) = self.distribution {
            interdist.update(duration_secs);
        } else {
            self.distribution = Some(
                InterArrivalDistribution::new(duration_secs, 0f64, 0.9f64)
            );
        }
        //let (m, v) = self.running_stats.unwrap();
        let (m, v) = self.distribution
                .as_ref()
                .map(|d| (d.mean(), d.variance()))
                .expect("distribution should have been constructed by now");

            println!("after update, mean: {}, var: {}", m, v);

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
                self.distribution.as_ref().and_then(|dist| {
                    let dur = at.duration_since(last_arrival);
                    let dur_secs = dur.as_secs() as f64 + dur.subsec_nanos() as f64 * 1e-9;
                    dist.cdf(dur_secs).map(|cdf| -(1.0 - cdf).log10())
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
        self.inter_arrival_window.as_ref().and_then(
            |iaw| iaw.phi(at),
        )
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
