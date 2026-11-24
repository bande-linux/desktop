use std::fs::File;
use std::os::fd::OwnedFd;
use std::os::unix::fs::FileExt;
use std::sync::{Mutex, PoisonError};

use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_feedback_v1::{
    self, TrancheFlags, ZwpLinuxDmabufFeedbackV1,
};

use super::State;

const TABLE_ENTRY: usize = 16;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Tranche {
    pub(crate) device: u64,
    pub(crate) formats: Vec<(u32, u64)>,
    pub(crate) scanout: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Feedback {
    pub(crate) main_device: Option<u64>,
    pub(crate) tranches: Vec<Tranche>,
}

impl Feedback {
    pub(crate) fn formats(&self) -> Vec<(u32, u64)> {
        let mut formats: Vec<(u32, u64)> = self
            .tranches
            .iter()
            .flat_map(|tranche| tranche.formats.iter().copied())
            .collect();
        formats.sort_unstable();
        formats.dedup();
        formats
    }

    pub(crate) fn modifiers(&self, fourcc: u32, ours: impl Fn(u64) -> bool) -> Vec<Vec<u64>> {
        let (scanout, rest): (Vec<&Tranche>, Vec<&Tranche>) = self
            .tranches
            .iter()
            .filter(|tranche| ours(tranche.device))
            .partition(|tranche| tranche.scanout);
        scanout
            .into_iter()
            .chain(rest)
            .map(|tranche| {
                tranche
                    .formats
                    .iter()
                    .filter(|(format, _)| *format == fourcc)
                    .map(|(_, modifier)| *modifier)
                    .collect::<Vec<u64>>()
            })
            .filter(|modifiers| !modifiers.is_empty())
            .collect()
    }
}

#[derive(Default)]
struct Reader {
    table: Vec<(u32, u64)>,
    feedback: Feedback,
    tranche: Tranche,
}

pub(crate) struct FeedbackData {
    surface: bool,
    reader: Mutex<Reader>,
}

impl FeedbackData {
    pub(crate) fn new(surface: bool) -> Self {
        Self {
            surface,
            reader: Mutex::new(Reader::default()),
        }
    }
}

fn device(bytes: &[u8]) -> Option<u64> {
    Some(u64::from_ne_bytes(bytes.try_into().ok()?))
}

fn table(fd: OwnedFd, size: u32) -> Vec<(u32, u64)> {
    let mut bytes = vec![0; size as usize];
    if let Err(error) = File::from(fd).read_exact_at(&mut bytes, 0) {
        tracing::warn!("cannot read the compositor's dma-buf format table: {error}");
        return Vec::new();
    }
    bytes
        .as_chunks::<TABLE_ENTRY>()
        .0
        .iter()
        .map(|entry| {
            let (format, rest) = entry.split_at(4);
            let format = u32::from_ne_bytes([format[0], format[1], format[2], format[3]]);
            let mut modifier = [0; 8];
            modifier.copy_from_slice(&rest[4..]);
            (format, u64::from_ne_bytes(modifier))
        })
        .collect()
}

impl Dispatch<ZwpLinuxDmabufFeedbackV1, FeedbackData> for State {
    fn event(
        state: &mut Self,
        _: &ZwpLinuxDmabufFeedbackV1,
        event: zwp_linux_dmabuf_feedback_v1::Event,
        data: &FeedbackData,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let mut reader = data.reader.lock().unwrap_or_else(PoisonError::into_inner);
        let reader = &mut *reader;
        match event {
            zwp_linux_dmabuf_feedback_v1::Event::FormatTable { fd, size } => {
                reader.table = table(fd, size);
            }
            zwp_linux_dmabuf_feedback_v1::Event::MainDevice { device: bytes } => {
                reader.feedback.main_device = device(&bytes);
            }
            zwp_linux_dmabuf_feedback_v1::Event::TrancheTargetDevice { device: bytes } => {
                reader.tranche.device = device(&bytes).unwrap_or_default();
            }
            zwp_linux_dmabuf_feedback_v1::Event::TrancheFormats { indices } => {
                let table = &reader.table;
                reader.tranche.formats.extend(
                    indices
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .filter_map(|index| table.get(usize::from(u16::from_ne_bytes(*index))))
                        .copied(),
                );
            }
            zwp_linux_dmabuf_feedback_v1::Event::TrancheFlags { flags } => {
                reader.tranche.scanout = flags
                    .into_result()
                    .is_ok_and(|flags| flags.contains(TrancheFlags::Scanout));
            }
            zwp_linux_dmabuf_feedback_v1::Event::TrancheDone => {
                let tranche = std::mem::take(&mut reader.tranche);
                reader.feedback.tranches.push(tranche);
            }
            zwp_linux_dmabuf_feedback_v1::Event::Done => {
                let feedback = std::mem::take(&mut reader.feedback);
                match data.surface {
                    true => {
                        state.feedback_changed |=
                            state.surface_feedback.as_ref() != Some(&feedback);
                        state.surface_feedback = Some(feedback);
                    }
                    false => {
                        state.dmabuf_formats = feedback.formats();
                        state.feedback_changed |=
                            state.default_feedback.as_ref() != Some(&feedback);
                        state.default_feedback = Some(feedback);
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AR24: u32 = u32::from_le_bytes(*b"AR24");
    const XR24: u32 = u32::from_le_bytes(*b"XR24");

    #[test]
    fn scanout_tranches_of_our_gpu_come_first_and_other_gpus_are_left_out() {
        let tranche = |device, scanout, modifiers: &[u64]| Tranche {
            device,
            formats: modifiers
                .iter()
                .flat_map(|&modifier| [(AR24, modifier), (XR24, modifier)])
                .collect(),
            scanout,
        };
        let feedback = Feedback {
            main_device: Some(1),
            tranches: vec![
                tranche(1, false, &[0, 7, 9]),
                tranche(2, true, &[0, 3]),
                tranche(1, true, &[7]),
            ],
        };
        assert_eq!(
            feedback.modifiers(AR24, |device| device == 1),
            vec![vec![7], vec![0, 7, 9]]
        );
    }
}
