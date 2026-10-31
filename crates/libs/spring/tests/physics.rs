#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::f64::consts::PI;
use std::time::{Duration, Instant};

use unbound_spring::{
    Animated, Animation, Decay, RUBBER_BAND_CONSTANT, Spring, SpringError, State, rubber_band,
};

const OMEGA: f64 = 12.0;
const TARGET: f64 = 3.0;
const RATIOS: [f64; 5] = [0.0, 0.3, 1.0, 1.0 + 1e-7, 2.5];

fn spring(damping_ratio: f64) -> Spring {
    Spring::new(OMEGA, damping_ratio).expect("valid spring")
}

fn released(position: f64, velocity: f64) -> State {
    State { position, velocity }
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance * (1.0 + a.abs().max(b.abs()))
}

#[test]
fn initial_conditions_hold_at_time_zero() {
    let start = released(-2.0, 7.5);
    for zeta in RATIOS {
        let state = spring(zeta).state_at(start, TARGET, 0.0);
        assert!(close(state.position, start.position, 1e-12), "zeta {zeta}");
        assert!(close(state.velocity, start.velocity, 1e-12), "zeta {zeta}");
    }
}

#[test]
fn trajectory_satisfies_the_damped_oscillator_equation() {
    let start = released(-2.0, 7.5);
    let h = 1e-4;
    for zeta in RATIOS {
        let spring = spring(zeta);
        let x = |t: f64| spring.state_at(start, TARGET, t).position;
        for step in 1..40 {
            let t = f64::from(step) * 0.025;
            let velocity = (x(t + h) - x(t - h)) / (2.0 * h);
            let acceleration = (x(t + h) - 2.0 * x(t) + x(t - h)) / (h * h);
            let residual =
                acceleration + 2.0 * zeta * OMEGA * velocity + OMEGA * OMEGA * (x(t) - TARGET);
            assert!(
                residual.abs() < 1e-3,
                "zeta {zeta} t {t}: residual {residual}"
            );
            let reported = spring.state_at(start, TARGET, t).velocity;
            assert!(
                close(reported, velocity, 1e-5),
                "zeta {zeta} t {t}: velocity {reported} vs {velocity}"
            );
        }
    }
}

#[test]
fn underdamped_motion_has_the_damped_period_and_logarithmic_decrement() {
    let zeta = 0.2;
    let spring = spring(zeta);
    let start = released(TARGET + 1.0, 0.0);
    let damped_period = 2.0 * PI / (OMEGA * (1.0 - zeta * zeta).sqrt());

    let dt = 1e-5;
    let mut upward_crossings = Vec::new();
    let mut previous = start.position - TARGET;
    let mut t = dt;
    while upward_crossings.len() < 3 {
        let displacement = spring.state_at(start, TARGET, t).position - TARGET;
        if previous < 0.0 && displacement >= 0.0 {
            upward_crossings.push(t);
        }
        previous = displacement;
        t += dt;
    }
    for pair in upward_crossings.windows(2) {
        assert!((pair[1] - pair[0] - damped_period).abs() < 2.0 * dt);
    }

    let peak = |cycles: f64| {
        spring
            .state_at(start, TARGET, cycles * damped_period)
            .position
            - TARGET
    };
    let decrement = (peak(0.0) / peak(1.0)).ln();
    let expected = 2.0 * PI * zeta / (1.0 - zeta * zeta).sqrt();
    assert!(
        close(decrement, expected, 1e-9),
        "{decrement} vs {expected}"
    );
}

#[test]
fn critically_and_overdamped_springs_released_at_rest_do_not_overshoot() {
    for zeta in [1.0, 1.0 + 1e-7, 1.5, 4.0] {
        let spring = spring(zeta);
        let start = released(TARGET - 5.0, 0.0);
        for step in 0..=2000 {
            let position = spring
                .state_at(start, TARGET, f64::from(step) * 0.001)
                .position;
            assert!(position <= TARGET, "zeta {zeta} overshot to {position}");
        }
    }
}

#[test]
fn energy_is_conserved_without_damping_and_never_grows_with_it() {
    let energy = |state: State| {
        let displacement = state.position - TARGET;
        0.5 * state.velocity * state.velocity + 0.5 * OMEGA * OMEGA * displacement * displacement
    };
    let start = released(-2.0, 7.5);
    let initial = energy(start);

    let undamped = spring(0.0);
    for step in 0..500 {
        let state = undamped.state_at(start, TARGET, f64::from(step) * 0.01);
        assert!(close(energy(state), initial, 1e-9));
    }

    for zeta in [0.3, 1.0, 2.5] {
        let damped = spring(zeta);
        let mut previous = initial;
        for step in 1..500 {
            let current = energy(damped.state_at(start, TARGET, f64::from(step) * 0.01));
            assert!(
                current <= previous * (1.0 + 1e-12),
                "zeta {zeta} step {step}"
            );
            previous = current;
        }
    }
}

#[test]
fn animation_state_does_not_depend_on_the_frame_rate() {
    let start = released(0.0, -40.0);
    let mut at_60_hz = Animation::new(spring(0.6), start, 100.0);
    let mut at_240_hz = at_60_hz;
    for _ in 0..60 {
        at_60_hz.advance(1.0 / 60.0);
    }
    for _ in 0..240 {
        at_240_hz.advance(1.0 / 240.0);
    }
    let (a, b) = (at_60_hz.state(), at_240_hz.state());
    assert!(close(a.position, b.position, 1e-9));
    assert!(close(a.velocity, b.velocity, 1e-9));
}

#[test]
fn retargeting_mid_flight_keeps_position_and_velocity_continuous() {
    let mut animation = Animation::new(spring(0.5), released(0.0, 0.0), 100.0);
    let before = animation.advance(0.08);
    animation.retarget(-50.0);
    let after = animation.state();
    assert_eq!(after, before);
    assert_eq!(animation.target(), -50.0);
    let next = animation.advance(0.001);
    assert!(next.velocity < before.velocity);
}

#[test]
fn animation_settles_on_its_target() {
    let mut animation = Animation::new(spring(1.0), released(0.0, 0.0), 1.0);
    assert!(!animation.is_at_rest(1e-3));
    animation.advance(2.0);
    assert!(animation.is_at_rest(1e-3));
}

#[test]
fn non_physical_parameters_are_rejected() {
    assert_eq!(
        Spring::new(0.0, 1.0),
        Err(SpringError::AngularFrequency(0.0))
    );
    assert_eq!(
        Spring::new(-1.0, 1.0),
        Err(SpringError::AngularFrequency(-1.0))
    );
    assert!(matches!(
        Spring::new(f64::NAN, 1.0),
        Err(SpringError::AngularFrequency(_))
    ));
    assert_eq!(Spring::new(1.0, -0.1), Err(SpringError::DampingRatio(-0.1)));
    assert!(matches!(
        Spring::new(1.0, f64::INFINITY),
        Err(SpringError::DampingRatio(_))
    ));
}

#[test]
fn presets_settle_to_a_thousandth_of_the_distance_in_their_named_times() {
    let settled_after = |spring: Spring, seconds: f64| {
        let mut animation = Animation::new(spring, released(0.0, 0.0), 1000.0);
        animation.advance(seconds);
        animation.is_at_rest(1.0)
    };
    for (spring, settles) in [
        (Spring::QUICK, 0.27),
        (Spring::STANDARD, 0.37),
        (Spring::GENTLE, 0.52),
        (Spring::FADE, 0.18),
    ] {
        assert!(settled_after(spring, settles * 1.05), "{spring:?}");
        assert!(!settled_after(spring, settles * 0.8), "{spring:?}");
    }
}

#[test]
fn momentum_overshoots_slightly_and_critical_presets_never() {
    let peak = |spring: Spring| {
        let mut animation = Animation::new(spring, released(0.0, 0.0), 1.0);
        (0..2000)
            .map(|_| animation.advance(0.001).position)
            .fold(f64::MIN, f64::max)
    };

    assert!((peak(Spring::MOMENTUM) - 1.006).abs() < 0.002);
    assert!((peak(Spring::BOUNCY) - 1.046).abs() < 0.005);
    assert!(peak(Spring::STANDARD) <= 1.0);
}

#[test]
fn decay_loses_its_rate_of_speed_every_millisecond_and_stops_at_its_projection() {
    let start = released(10.0, 2000.0);
    let decay = Decay::NORMAL;
    let after = decay.state_at(start, 0.001);
    let rate = after.velocity / start.velocity;
    assert!(rate > 0.98 && rate < 1.0);
    let settled = decay.state_at(start, 20.0);
    assert!(close(settled.position, decay.project(start), 1e-9));
    assert!(close(
        Decay::NORMAL.state_at(start, 0.001).velocity,
        2000.0 * 0.998,
        1e-9
    ));
}

#[test]
fn rubber_band_resists_more_the_further_it_is_pulled() {
    let dimension = 400.0;
    assert!(close(
        rubber_band(1.0, dimension),
        RUBBER_BAND_CONSTANT,
        1e-2
    ));
    assert!(close(
        rubber_band(-50.0, dimension),
        -rubber_band(50.0, dimension),
        1e-12
    ));
    let pulls: Vec<f64> = [0.0, 100.0, 200.0, 400.0, 10_000.0]
        .iter()
        .map(|pull| rubber_band(*pull, dimension))
        .collect();
    assert!(pulls.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(pulls[4] < dimension);
    assert!(pulls[2] - pulls[1] < pulls[1] - pulls[0]);
}

#[test]
fn a_glide_stops_asking_for_frames_once_the_rest_of_its_motion_is_invisible() {
    let start = Instant::now();
    let refresh = Duration::from_micros(8333);
    let settled_after = |mut animated: Animated<Instant>, target: f32| {
        animated.animate_at(start, false, target, Spring::QUICK);
        let mut last_moving = 0.0;
        for frame in 1..1000 {
            let (value, moving) = animated.value_at(start + refresh * frame);
            if !moving {
                assert_eq!(value, target);
                return target - last_moving;
            }
            last_moving = value;
        }
        panic!("the animation never settled");
    };
    let snap = settled_after(Animated::points(0.0), 100.0);
    assert!(
        (0.005..0.1).contains(&snap),
        "a glide stops once the rest of its motion is a fraction of a pixel: {snap}"
    );
    let step = settled_after(Animated::new(0.0), 1.0);
    assert!(
        (0.0002..1.0 / 255.0).contains(&step),
        "a fade stops once the rest of it is below one shade: {step}"
    );
}
