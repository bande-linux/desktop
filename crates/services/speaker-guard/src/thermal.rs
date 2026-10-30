#[derive(Debug, Clone, PartialEq)]
pub struct ThermalParams {
    pub ambient: f64,
    pub coil_resistance: f64,
    pub coil_time_constant: f64,
    pub magnet_resistance: f64,
    pub magnet_time_constant: f64,
    pub limit: f64,
    pub headroom: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Temperatures {
    pub coil: f64,
    pub magnet: f64,
}

impl ThermalParams {
    pub fn max_continuous_power(&self) -> f64 {
        (self.limit - self.ambient) / (self.coil_resistance + self.magnet_resistance)
    }

    pub fn worst_case(&self) -> Temperatures {
        Temperatures {
            coil: self.limit,
            magnet: self.ambient + self.max_continuous_power() * self.magnet_resistance,
        }
    }

    pub fn advance(&self, from: Temperatures, power: f64, seconds: f64) -> Temperatures {
        let magnet_target = self.ambient + power * self.magnet_resistance;
        let magnet_offset = from.magnet - magnet_target;
        let magnet_decay = (-seconds / self.magnet_time_constant).exp();
        let coil_target = magnet_target + power * self.coil_resistance;
        let coupled = magnet_offset * self.magnet_time_constant
            / (self.magnet_time_constant - self.coil_time_constant);
        let coil_decay = (-seconds / self.coil_time_constant).exp();
        Temperatures {
            coil: coil_target
                + coupled * magnet_decay
                + (from.coil - coil_target - coupled) * coil_decay,
            magnet: magnet_target + magnet_offset * magnet_decay,
        }
    }

    pub fn allowed_power(&self, now: Temperatures) -> f64 {
        ((self.limit - now.magnet) / self.coil_resistance).max(0.0)
    }

    pub fn exceeds_hard_limit(&self, now: Temperatures) -> bool {
        let hard = self.limit + self.headroom;
        now.coil > hard || now.magnet > hard
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LimiterParams {
    pub attack: f64,
    pub release: f64,
    pub max_reduction_db: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Protection {
    pub thermal: ThermalParams,
    pub limiter: LimiterParams,
    pub temperatures: Temperatures,
    pub gain_db: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict {
    Gain(f64),
    HardLimit(Temperatures),
}

impl Protection {
    pub fn update(&mut self, applied_power: f64, seconds: f64) -> Verdict {
        self.temperatures = self
            .thermal
            .advance(self.temperatures, applied_power, seconds);
        if self.thermal.exceeds_hard_limit(self.temperatures) {
            return Verdict::HardLimit(self.temperatures);
        }
        let unattenuated = applied_power / db_to_power(self.gain_db);
        let target = self.target_db(unattenuated);
        let at_current = unattenuated * db_to_power(self.gain_db);
        let predicted = self.thermal.advance(self.temperatures, at_current, seconds);
        let time_constant = if target >= self.gain_db {
            self.limiter.release
        } else if predicted.coil > self.thermal.limit {
            0.0
        } else {
            self.limiter.attack
        };
        let step = if time_constant > 0.0 {
            1.0 - (-seconds / time_constant).exp()
        } else {
            1.0
        };
        self.gain_db = (self.gain_db + (target - self.gain_db) * step)
            .clamp(-self.limiter.max_reduction_db, 0.0);
        Verdict::Gain(self.gain_db)
    }

    fn target_db(&self, unattenuated_power: f64) -> f64 {
        let allowed = self.thermal.allowed_power(self.temperatures);
        if unattenuated_power <= allowed {
            0.0
        } else if allowed <= 0.0 {
            -self.limiter.max_reduction_db
        } else {
            (10.0 * (allowed / unattenuated_power).log10()).max(-self.limiter.max_reduction_db)
        }
    }
}

pub fn db_to_power(db: f64) -> f64 {
    10f64.powf(db / 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn j473() -> ThermalParams {
        ThermalParams {
            ambient: 46.0,
            coil_resistance: 40.0,
            coil_time_constant: 3.7,
            magnet_resistance: 60.0,
            magnet_time_constant: 250.0,
            limit: 140.0,
            headroom: 40.0,
        }
    }

    fn euler(params: &ThermalParams, from: Temperatures, power: f64, seconds: f64) -> Temperatures {
        let steps = (seconds * 10_000.0) as usize;
        let dt = seconds / steps as f64;
        (0..steps).fold(from, |t, _| Temperatures {
            coil: t.coil
                + dt * (t.magnet + power * params.coil_resistance - t.coil)
                    / params.coil_time_constant,
            magnet: t.magnet
                + dt * (params.ambient + power * params.magnet_resistance - t.magnet)
                    / params.magnet_time_constant,
        })
    }

    #[test]
    fn closed_form_matches_the_two_pole_network_it_protects_with() {
        let params = j473();
        let start = Temperatures {
            coil: 90.0,
            magnet: 60.0,
        };
        for (power, seconds) in [(2.0, 1.5), (0.5, 40.0), (0.0, 300.0)] {
            let exact = params.advance(start, power, seconds);
            let numeric = euler(&params, start, power, seconds);
            assert!(
                (exact.coil - numeric.coil).abs() < 0.05,
                "{exact:?} {numeric:?}"
            );
            assert!(
                (exact.magnet - numeric.magnet).abs() < 0.05,
                "{exact:?} {numeric:?}"
            );
        }
    }

    #[test]
    fn limiter_keeps_the_coil_under_its_hard_limit_at_full_scale() {
        let mut protection = Protection {
            thermal: j473(),
            limiter: LimiterParams {
                attack: 1.0,
                release: 20.0,
                max_reduction_db: 20.0,
            },
            temperatures: Temperatures {
                coil: 46.0,
                magnet: 46.0,
            },
            gain_db: 0.0,
        };
        let full_scale_sine_power = (15.87_f64 / 2.0_f64.sqrt()).powi(2) / 4.6;
        let period = 1024.0 / 48_000.0;
        let mut hottest: f64 = 0.0;
        for _ in 0..(1800.0 / period) as usize {
            let applied = full_scale_sine_power * db_to_power(protection.gain_db);
            let verdict = protection.update(applied, period);
            assert!(matches!(verdict, Verdict::Gain(_)), "{verdict:?}");
            hottest = hottest.max(protection.temperatures.coil);
        }
        let limit = protection.thermal.limit;
        assert!(hottest < limit + 5.0, "peaked at {hottest}");
        assert!(protection.temperatures.coil <= limit + 1.0);
    }
}
