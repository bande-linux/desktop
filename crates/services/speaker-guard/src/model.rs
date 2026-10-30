use std::path::Path;

use kdl::{KdlDocument, KdlNode};

use crate::sense::SenseLayout;
use crate::thermal::{LimiterParams, ThermalParams};

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub card: String,
    pub sense_device: u32,
    pub period_frames: usize,
    pub amplifier_sensitivity: f64,
    pub max_gain_db: f64,
    pub speakers: Vec<Speaker>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Speaker {
    pub control_prefix: String,
    pub sense: SenseLayout,
    pub thermal: ThermalParams,
    pub limiter: LimiterParams,
}

#[derive(Debug)]
pub enum ModelError {
    Read(std::io::Error),
    Parse(kdl::KdlError),
    Missing(String),
    Invalid(String),
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Read(error) => write!(f, "cannot read the model file: {error}"),
            Self::Parse(error) => write!(f, "cannot parse the model file: {error}"),
            Self::Missing(key) => write!(f, "the model file has no {key}"),
            Self::Invalid(reason) => write!(f, "the model file is invalid: {reason}"),
        }
    }
}

impl std::error::Error for ModelError {}

fn number(document: &KdlDocument, key: &str) -> Result<f64, ModelError> {
    document
        .get_arg(key)
        .and_then(|value| {
            value
                .as_float()
                .or_else(|| value.as_integer().map(|i| i as f64))
        })
        .ok_or_else(|| ModelError::Missing(key.to_owned()))
}

fn count(document: &KdlDocument, key: &str) -> Result<usize, ModelError> {
    document
        .get_arg(key)
        .and_then(|value| value.as_integer())
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| ModelError::Missing(key.to_owned()))
}

fn counts(document: &KdlDocument, key: &str) -> Result<Vec<usize>, ModelError> {
    let node = document
        .get(key)
        .ok_or_else(|| ModelError::Missing(key.to_owned()))?;
    node.entries()
        .iter()
        .map(|entry| {
            entry
                .value()
                .as_integer()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| ModelError::Invalid(key.to_owned()))
        })
        .collect()
}

fn text(document: &KdlDocument, key: &str) -> Result<String, ModelError> {
    document
        .get_arg(key)
        .and_then(|value| value.as_string())
        .map(str::to_owned)
        .ok_or_else(|| ModelError::Missing(key.to_owned()))
}

fn speaker(node: &KdlNode, document: &KdlDocument) -> Result<Speaker, ModelError> {
    let control_prefix = node
        .entries()
        .first()
        .and_then(|entry| entry.value().as_string())
        .ok_or_else(|| ModelError::Missing("speaker control prefix".to_owned()))?
        .to_owned();
    let settings = node
        .children()
        .ok_or_else(|| ModelError::Missing("speaker settings".to_owned()))?;
    let speaker = Speaker {
        control_prefix,
        sense: SenseLayout {
            channels: count(document, "sense-channels")?,
            idle_channels: counts(document, "sense-idle-channels")?,
            current_channel: count(settings, "current-channel")?,
            voltage_channel: count(settings, "voltage-channel")?,
            current_full_scale: number(document, "current-full-scale")?,
            voltage_full_scale: number(document, "voltage-full-scale")?,
            dc_resistance: number(settings, "dc-resistance")?,
        },
        thermal: ThermalParams {
            ambient: number(settings, "ambient")?,
            coil_resistance: number(settings, "coil-thermal-resistance")?,
            coil_time_constant: number(settings, "coil-time-constant")?,
            magnet_resistance: number(settings, "magnet-thermal-resistance")?,
            magnet_time_constant: number(settings, "magnet-time-constant")?,
            limit: number(settings, "temperature-limit")?,
            headroom: number(settings, "hard-limit-headroom")?,
        },
        limiter: LimiterParams {
            attack: number(settings, "attack")?,
            release: number(settings, "release")?,
            max_reduction_db: number(settings, "max-reduction")?,
        },
    };
    let sense = &speaker.sense;
    let thermal = &speaker.thermal;
    let limiter = &speaker.limiter;
    let positive = [
        sense.current_full_scale,
        sense.voltage_full_scale,
        sense.dc_resistance,
        thermal.coil_resistance,
        thermal.magnet_resistance,
        thermal.coil_time_constant,
        thermal.magnet_time_constant,
        limiter.release,
        limiter.max_reduction_db,
    ];
    let non_negative = [thermal.headroom, limiter.attack];
    if !positive.iter().all(|v| v.is_finite() && *v > 0.0)
        || !non_negative.iter().all(|v| v.is_finite() && *v >= 0.0)
        || !thermal.ambient.is_finite()
        || !thermal.limit.is_finite()
        || limiter.max_reduction_db > 100.0
    {
        return Err(ModelError::Invalid("speaker parameters".to_owned()));
    }
    if sense.current_channel >= sense.channels
        || sense.voltage_channel >= sense.channels
        || sense.current_channel == sense.voltage_channel
        || sense.idle_channels.is_empty()
        || sense.idle_channels.iter().any(|&c| {
            c >= sense.channels || c == sense.current_channel || c == sense.voltage_channel
        })
    {
        return Err(ModelError::Invalid("sense channels".to_owned()));
    }
    if thermal.magnet_time_constant <= thermal.coil_time_constant
        || thermal.coil_time_constant <= 0.0
        || thermal.limit <= thermal.ambient
        || sense.dc_resistance <= 0.0
    {
        return Err(ModelError::Invalid("thermal parameters".to_owned()));
    }
    Ok(speaker)
}

impl Model {
    pub fn parse(source: &str) -> Result<Self, ModelError> {
        let document: KdlDocument = source.parse().map_err(ModelError::Parse)?;
        let speakers = document
            .nodes()
            .iter()
            .filter(|node| node.name().value() == "speaker")
            .map(|node| speaker(node, &document))
            .collect::<Result<Vec<_>, _>>()?;
        if speakers.is_empty() {
            return Err(ModelError::Missing("speaker".to_owned()));
        }
        let max_gain_db = number(&document, "max-gain")?;
        let amplifier_sensitivity = number(&document, "amplifier-sensitivity")?;
        if !(amplifier_sensitivity.is_finite() && amplifier_sensitivity > 0.0) {
            return Err(ModelError::Invalid("amplifier-sensitivity".to_owned()));
        }
        let period_frames = count(&document, "period-frames")?;
        if period_frames == 0 {
            return Err(ModelError::Invalid("period-frames".to_owned()));
        }
        if !(-100.0..=0.0).contains(&max_gain_db) {
            return Err(ModelError::Invalid("max-gain".to_owned()));
        }
        Ok(Self {
            card: text(&document, "card")?,
            sense_device: u32::try_from(count(&document, "sense-device")?)
                .map_err(|_| ModelError::Invalid("sense-device".to_owned()))?,
            period_frames,
            amplifier_sensitivity,
            max_gain_db,
            speakers,
        })
    }

    pub fn load(path: &Path) -> Result<Self, ModelError> {
        Self::parse(&std::fs::read_to_string(path).map_err(ModelError::Read)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const J473: &str = include_str!("../data/j473.kdl");

    #[test]
    fn the_shipped_j473_model_parses_and_caps_the_gain_until_the_sense_scale_is_validated() {
        let model = Model::parse(J473).unwrap();
        assert_eq!(model.speakers.len(), 1);
        assert_eq!(model.max_gain_db, -20.0);
    }

    #[test]
    fn parameters_that_would_disable_the_thermal_limit_are_rejected() {
        for (from, to) in [
            ("max-reduction 20.0", "max-reduction -20.0"),
            ("amplifier-sensitivity 15.87", "amplifier-sensitivity 0.0"),
            (
                "coil-thermal-resistance 40.0",
                "coil-thermal-resistance 0.0",
            ),
            ("voltage-full-scale 14.0", "voltage-full-scale 0.0"),
            ("hard-limit-headroom 40.0", "hard-limit-headroom -1.0"),
            ("period-frames 1024", "period-frames 0"),
        ] {
            let source = J473.replace(from, to);
            assert_ne!(source, J473, "{from}");
            assert!(
                matches!(Model::parse(&source), Err(ModelError::Invalid(_))),
                "{to}"
            );
        }
    }

    #[test]
    fn a_max_gain_above_zero_is_rejected() {
        let source = J473.replace("max-gain -20.0", "max-gain 3.0");
        assert!(matches!(Model::parse(&source), Err(ModelError::Invalid(_))));
        let source = J473.replace("max-gain -20.0\n", "");
        assert!(matches!(Model::parse(&source), Err(ModelError::Missing(_))));
    }
}
