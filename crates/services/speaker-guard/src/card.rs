use std::ffi::CString;

use alsa::ctl::{Ctl, ElemId, ElemIface, ElemType, ElemValue};
use alsa::mixer::MilliBel;
use alsa::pcm::{Access, Format, HwParams, PCM};
use alsa::{Direction, Round, ValueOr};

use crate::model::Model;

const UNLOCK_MAGIC: i32 = 0xdec1_be15_u32 as i32;
const AMPLIFIER_SENSITIVITY_TOLERANCE: f64 = 0.03;
const EAGAIN: i32 = 11;

#[derive(Debug)]
pub enum CardError {
    Alsa(&'static str, String, alsa::Error),
    Readback(String),
    Sensitivity { configured: f64, programmed: f64 },
}

impl std::fmt::Display for CardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Alsa(action, what, error) => write!(f, "cannot {action} {what}: {error}"),
            Self::Readback(what) => write!(f, "{what} did not keep the value written to it"),
            Self::Sensitivity {
                configured,
                programmed,
            } => write!(
                f,
                "the amplifier is set to {programmed:.2} V/FS, the model needs {configured:.2} V/FS"
            ),
        }
    }
}

impl std::error::Error for CardError {}

impl CardError {
    pub fn errno(&self) -> Option<i32> {
        match self {
            Self::Alsa(_, _, error) => Some(error.errno()),
            _ => None,
        }
    }
}

struct Control {
    name: String,
    id: ElemId,
    value: ElemValue,
}

impl Control {
    fn new(ctl: &Ctl, name: String, kind: ElemType) -> Result<Self, CardError> {
        let mut id = ElemId::new(ElemIface::Mixer);
        let cname = CString::new(name.clone()).map_err(|_| CardError::Readback(name.clone()))?;
        id.set_name(&cname);
        let mut value =
            ElemValue::new(kind).map_err(|e| CardError::Alsa("allocate", name.clone(), e))?;
        value.set_id(&id);
        ctl.elem_read(&mut value)
            .map_err(|e| CardError::Alsa("read", name.clone(), e))?;
        Ok(Self { name, id, value })
    }

    fn locked(ctl: &Ctl, name: String, kind: ElemType) -> Result<Self, CardError> {
        let mut control = Self::new(ctl, name, kind)?;
        ctl.elem_lock(&control.id)
            .map_err(|e| CardError::Alsa("lock", control.name.clone(), e))?;
        control.read(ctl)?;
        Ok(control)
    }

    fn write(&mut self, ctl: &Ctl) -> Result<(), CardError> {
        ctl.elem_write(&self.value)
            .map_err(|e| CardError::Alsa("write", self.name.clone(), e))
    }

    fn read(&mut self, ctl: &Ctl) -> Result<(), CardError> {
        ctl.elem_read(&mut self.value)
            .map_err(|e| CardError::Alsa("read", self.name.clone(), e))
    }

    fn set_switch(&mut self, ctl: &Ctl, on: bool) -> Result<(), CardError> {
        self.value.set_boolean(0, on);
        self.write(ctl)?;
        self.read(ctl)?;
        match self.value.get_boolean(0) {
            Some(state) if state == on => Ok(()),
            _ => Err(CardError::Readback(self.name.clone())),
        }
    }

    fn integer(&self) -> Result<i32, CardError> {
        self.value
            .get_integer(0)
            .ok_or_else(|| CardError::Readback(self.name.clone()))
    }

    fn set_integer(&mut self, ctl: &Ctl, value: i32) -> Result<(), CardError> {
        self.value.set_integer(0, value);
        self.write(ctl)
    }
}

struct SpeakerControls {
    volume: Control,
    applied_step: Option<i64>,
}

pub struct Card {
    ctl: Ctl,
    unlock: Control,
    speakers: Vec<SpeakerControls>,
}

fn decibels_to_sensitivity(decibels: f64) -> f64 {
    10f64.powf(decibels / 20.0) * std::f64::consts::SQRT_2
}

impl Card {
    pub fn open(model: &Model) -> Result<Self, CardError> {
        let device = format!("hw:{}", model.card);
        let ctl =
            Ctl::new(&device, false).map_err(|e| CardError::Alsa("open", device.clone(), e))?;
        let unlock = Control::locked(&ctl, "Speaker Volume Unlock".to_owned(), ElemType::Integer)?;
        let speakers = model
            .speakers
            .iter()
            .map(|speaker| {
                let named = |control: &str| format!("{}{control}", speaker.control_prefix);
                let mut voltage_sense =
                    Control::locked(&ctl, named("VSENSE Switch"), ElemType::Boolean)?;
                voltage_sense.set_switch(&ctl, true)?;
                let mut current_sense =
                    Control::locked(&ctl, named("ISENSE Switch"), ElemType::Boolean)?;
                current_sense.set_switch(&ctl, true)?;
                let amplifier_gain =
                    Control::locked(&ctl, named("Amp Gain Volume"), ElemType::Integer)?;
                let programmed = ctl
                    .convert_to_db(&amplifier_gain.id, i64::from(amplifier_gain.integer()?))
                    .map_err(|e| CardError::Alsa("convert", amplifier_gain.name.clone(), e))?;
                let programmed = decibels_to_sensitivity(f64::from(programmed.to_db()));
                let configured = model.amplifier_sensitivity;
                if ((programmed - configured) / configured).abs() > AMPLIFIER_SENSITIVITY_TOLERANCE
                {
                    return Err(CardError::Sensitivity {
                        configured,
                        programmed,
                    });
                }
                let volume = Control::locked(&ctl, named("Speaker Volume"), ElemType::Integer)?;
                Ok(SpeakerControls {
                    volume,
                    applied_step: None,
                })
            })
            .collect::<Result<Vec<_>, CardError>>()?;
        Ok(Self {
            ctl,
            unlock,
            speakers,
        })
    }

    pub fn heartbeat(&mut self) -> Result<(), CardError> {
        self.unlock.set_integer(&self.ctl, UNLOCK_MAGIC)
    }

    pub fn set_gain(&mut self, speaker: usize, gain_db: f64) -> Result<(), CardError> {
        let Some(controls) = self.speakers.get_mut(speaker) else {
            return Ok(());
        };
        let step = self
            .ctl
            .convert_from_db(
                &controls.volume.id,
                MilliBel((gain_db * 100.0).floor() as i64),
                Round::Floor,
            )
            .map_err(|e| CardError::Alsa("convert", controls.volume.name.clone(), e))?;
        if controls.applied_step == Some(step) {
            return Ok(());
        }
        controls
            .volume
            .set_integer(&self.ctl, i32::try_from(step).unwrap_or(0))?;
        controls.applied_step = Some(step);
        Ok(())
    }
}

pub struct SensePcm {
    pcm: PCM,
    name: String,
}

impl SensePcm {
    pub fn open(model: &Model, channels: usize) -> Result<Self, CardError> {
        let name = format!("hw:{},{}", model.card, model.sense_device);
        let pcm = PCM::new(&name, Direction::Capture, true)
            .map_err(|e| CardError::Alsa("open", name.clone(), e))?;
        {
            let params =
                HwParams::any(&pcm).map_err(|e| CardError::Alsa("query", name.clone(), e))?;
            let configure = |result: alsa::Result<()>| {
                result.map_err(|e| CardError::Alsa("configure", name.clone(), e))
            };
            configure(params.set_access(Access::RWInterleaved))?;
            configure(params.set_format(Format::s16()))?;
            configure(params.set_channels(u32::try_from(channels).unwrap_or(0)))?;
            configure(params.set_rate_near(48_000, ValueOr::Nearest).map(|_| ()))?;
            let period = alsa::pcm::Frames::try_from(model.period_frames).unwrap_or(1024);
            configure(
                params
                    .set_period_size_near(period, ValueOr::Nearest)
                    .map(|_| ()),
            )?;
            configure(params.set_buffer_size_near(period * 4).map(|_| ()))?;
            configure(pcm.hw_params(&params))?;
        }
        pcm.prepare()
            .map_err(|e| CardError::Alsa("prepare", name.clone(), e))?;
        pcm.start()
            .map_err(|e| CardError::Alsa("start", name.clone(), e))?;
        Ok(Self { pcm, name })
    }

    pub fn read(&self, buffer: &mut [i16], timeout_ms: u32) -> Result<Option<usize>, CardError> {
        let ready = self
            .pcm
            .wait(Some(timeout_ms))
            .map_err(|e| CardError::Alsa("wait for", self.name.clone(), e))?;
        if !ready {
            return Ok(None);
        }
        let io = self
            .pcm
            .io_i16()
            .map_err(|e| CardError::Alsa("read", self.name.clone(), e))?;
        match io.readi(buffer) {
            Ok(frames) => Ok(Some(frames)),
            Err(error) if error.errno() == EAGAIN => Ok(Some(0)),
            Err(error) => Err(CardError::Alsa("read", self.name.clone(), error)),
        }
    }

    pub fn restart(&self) -> Result<(), CardError> {
        self.pcm
            .prepare()
            .and_then(|()| self.pcm.start())
            .map_err(|e| CardError::Alsa("restart", self.name.clone(), e))
    }
}
