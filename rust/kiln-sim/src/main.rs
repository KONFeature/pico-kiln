//! Run a firing on the host and print the transcript — the real control loop
//! against the software thermal model. See the library crate for the harness.

use kiln_core::profile::{Profile, Step};
use kiln_sim::{simulate, ThermalModel};

fn main() {
    // Heat to a 150 °C hold and track it — closed-loop setpoint tracking.
    let profile = Profile::new(&[Step::hold(150.0, 1.0e9)]).unwrap();
    let result = simulate("demo", profile, 8_000, ThermalModel::kiln());

    println!("  t(min)    temp   target    ssr%   state");
    let last_index = result.samples.len() - 1;
    for (i, s) in result.samples.iter().enumerate() {
        if i % 60 == 0 || i == last_index {
            println!(
                "  {:>6.1}  {:>6.1}  {:>6.1}  {:>6.1}   {:?}",
                s.t_s / 60.0,
                s.temp,
                s.target,
                s.ssr,
                s.state
            );
        }
    }

    let last = &result.samples[last_index];
    println!(
        "\nsettled: temp={:.1}C target={:.1}C  watchdog_feeds={}",
        last.temp, last.target, result.watchdog_feeds
    );
}
