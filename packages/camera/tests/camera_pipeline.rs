//! Mock H.264 source → camera messages → decoder → sink, checking the
//! decoded frame dimensions.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use freelife_camera::{
    CameraEvent, CameraReceiver, DecodedFrame, FrameSink, ReceiverConfig, SinkError,
};
use freelife_core::{ActiveConnection, FreeLifeMessage, Identity};
use tokio::sync::mpsc;

const WIDTH: u32 = 320;
const HEIGHT: u32 = 240;
const FIXTURE: &[u8] = include_bytes!("fixtures/testsrc_320x240.h264");

/// Split an Annex B byte stream into access units (one coded picture each).
///
/// A picture starts at a slice whose `first_mb_in_slice` is 0, which is the
/// first bit of its RBSP being 1 (`ue(v)` codes 0 as a single `1` bit).
/// Parameter sets (SPS/PPS) go with the picture that follows them.
fn access_units(stream: &[u8]) -> Vec<Vec<u8>> {
    let starts: Vec<usize> = (0..stream.len().saturating_sub(3))
        .filter(|&i| stream[i..].starts_with(&[0, 0, 1]))
        .collect();

    let mut units: Vec<Vec<u8>> = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    let mut current_has_slice = false;
    for (n, &start) in starts.iter().enumerate() {
        let end = starts.get(n + 1).map_or(stream.len(), |next| {
            // The next start code may carry a leading zero (4-byte form).
            if *next > 0 && stream[*next - 1] == 0 {
                *next - 1
            } else {
                *next
            }
        });
        let begin = if start > 0 && stream[start - 1] == 0 {
            start - 1
        } else {
            start
        };
        let nal = &stream[begin..end];
        let nal_type = stream[start + 3] & 0x1f;
        let is_slice = nal_type == 1 || nal_type == 5;
        let first_slice_of_picture = is_slice && stream[start + 4] & 0x80 != 0;
        let parameter_set = nal_type == 7 || nal_type == 8;

        if current_has_slice && (first_slice_of_picture || parameter_set) {
            units.push(std::mem::take(&mut current));
            current_has_slice = false;
        }
        current.extend_from_slice(nal);
        current_has_slice |= is_slice;
    }
    if !current.is_empty() {
        units.push(current);
    }
    units
}

/// Records what reaches the virtual camera.
#[derive(Clone, Default)]
struct RecordingSink {
    frames: Arc<Mutex<Vec<(u32, u32, usize)>>>,
    configured: Arc<Mutex<Vec<(u32, u32)>>>,
}

impl FrameSink for RecordingSink {
    fn configure(&mut self, width: u32, height: u32) -> Result<(), SinkError> {
        self.configured.lock().unwrap().push((width, height));
        Ok(())
    }

    fn write(&mut self, frame: &DecodedFrame) -> Result<(), SinkError> {
        self.frames
            .lock()
            .unwrap()
            .push((frame.width, frame.height, frame.i420.len()));
        Ok(())
    }

    fn describe(&self) -> String {
        "recording".into()
    }
}

/// A phone-side source that sends the access units as `CameraFrame`s
/// through a channel standing in for a FreeLife session.
fn mock_source(units: Vec<Vec<u8>>) -> mpsc::Receiver<FreeLifeMessage> {
    let (tx, rx) = mpsc::channel(256);
    tokio::spawn(async move {
        tx.send(FreeLifeMessage::CameraStreamInfo {
            width: WIDTH,
            height: HEIGHT,
            fps: 30,
        })
        .await
        .unwrap();
        for (seq, unit) in units.into_iter().enumerate() {
            tx.send(FreeLifeMessage::CameraFrame {
                seq: seq as u64,
                timestamp_us: 1_700_000_000_000_000 + seq as u64 * 33_333,
                keyframe: seq == 0,
                data: unit,
            })
            .await
            .unwrap();
        }
        tx.send(FreeLifeMessage::CameraEnded).await.unwrap();
    });
    rx
}

#[test]
fn fixture_splits_into_pictures() {
    let units = access_units(FIXTURE);
    // 12 pictures; the first carries the SPS and PPS with its keyframe.
    assert_eq!(units.len(), 12, "got {} access units", units.len());
    assert_eq!(units[0][4] & 0x1f, 7, "first unit starts with the SPS");
}

#[tokio::test]
async fn mock_h264_source_decodes_to_the_right_dimensions() {
    let sink = RecordingSink::default();
    let (mut receiver, mut events) = CameraReceiver::new(
        Box::new(sink.clone()),
        ReceiverConfig {
            report_interval: Duration::from_millis(1),
            preview_width: 160,
        },
    )
    .expect("H.264 decoder");

    let mut source = mock_source(access_units(FIXTURE));
    while let Some(msg) = source.recv().await {
        receiver.handle(&msg);
    }

    let frames = sink.frames.lock().unwrap().clone();
    assert_eq!(frames.len(), 12, "every picture should decode");
    let i420_len = DecodedFrame::i420_len(WIDTH, HEIGHT);
    for (w, h, len) in &frames {
        assert_eq!((*w, *h), (WIDTH, HEIGHT));
        assert_eq!(*len, i420_len, "I420 is 1.5 bytes per pixel, unpadded");
    }
    assert_eq!(*sink.configured.lock().unwrap(), vec![(WIDTH, HEIGHT)]);

    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        seen.push(event);
    }
    assert!(seen.contains(&CameraEvent::Started {
        width: WIDTH,
        height: HEIGHT
    }));
    assert!(seen.contains(&CameraEvent::Ended));
    let preview = seen
        .iter()
        .find_map(|e| match e {
            CameraEvent::Preview { png_base64 } => Some(png_base64.clone()),
            _ => None,
        })
        .expect("a preview was published");
    let png = base64_decode(&preview);
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    let decoded = image::load_from_memory(&png).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (160, 120));
}

#[tokio::test]
async fn garbage_is_counted_not_fatal() {
    let sink = RecordingSink::default();
    let (mut receiver, _events) =
        CameraReceiver::new(Box::new(sink.clone()), ReceiverConfig::default()).unwrap();
    receiver.handle(&FreeLifeMessage::CameraFrame {
        seq: 0,
        timestamp_us: 1,
        keyframe: false,
        data: vec![0xde, 0xad, 0xbe, 0xef],
    });
    // A real stream still works afterwards.
    for (seq, unit) in access_units(FIXTURE).into_iter().enumerate() {
        receiver.handle(&FreeLifeMessage::CameraFrame {
            seq: seq as u64 + 1,
            timestamp_us: 2,
            keyframe: false,
            data: unit,
        });
    }
    assert_eq!(sink.frames.lock().unwrap().len(), 12);
}

#[tokio::test]
async fn frames_travel_over_a_connection_handle() {
    // The phone's path: frames sent through an ActiveConnection arrive at the
    // desktop receiver in order, and congestion shows up as `false`, not a block.
    let (tx, mut rx) = mpsc::channel(256);
    let conn = ActiveConnection::from_sender(Identity::generate().unwrap().device_id(), tx);
    for (seq, unit) in access_units(FIXTURE).into_iter().enumerate() {
        assert!(conn
            .try_send(FreeLifeMessage::CameraFrame {
                seq: seq as u64,
                timestamp_us: 0,
                keyframe: seq == 0,
                data: unit,
            })
            .unwrap());
    }
    drop(conn);

    let sink = RecordingSink::default();
    let (mut receiver, _events) =
        CameraReceiver::new(Box::new(sink.clone()), ReceiverConfig::default()).unwrap();
    while let Some(msg) = rx.recv().await {
        receiver.handle(&msg);
    }
    assert_eq!(sink.frames.lock().unwrap().len(), 12);
}

#[test]
fn sink_failures_are_reported_as_events() {
    struct Broken;
    impl FrameSink for Broken {
        fn configure(&mut self, _: u32, _: u32) -> Result<(), SinkError> {
            Err(SinkError::Unsupported("no camera".into()))
        }
        fn write(&mut self, _: &DecodedFrame) -> Result<(), SinkError> {
            unreachable!("configure fails first")
        }
        fn describe(&self) -> String {
            "broken".into()
        }
    }
    let (mut receiver, mut events) =
        CameraReceiver::new(Box::new(Broken), ReceiverConfig::default()).unwrap();
    for unit in access_units(FIXTURE).into_iter().take(1) {
        receiver.handle(&FreeLifeMessage::CameraFrame {
            seq: 0,
            timestamp_us: 0,
            keyframe: true,
            data: unit,
        });
    }
    let mut errors = 0;
    while let Ok(e) = events.try_recv() {
        if matches!(e, CameraEvent::SinkError { .. }) {
            errors += 1;
        }
    }
    assert!(errors >= 1);
}

fn base64_decode(s: &str) -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(s).unwrap()
}

#[cfg(target_os = "linux")]
#[test]
fn missing_loopback_device_explains_the_fix() {
    use freelife_camera::sink::V4l2LoopbackSink;
    use freelife_camera::FrameSink;

    let mut sink = V4l2LoopbackSink::new("/dev/freelife-does-not-exist");
    let err = sink.configure(320, 240).unwrap_err().to_string();
    assert!(err.contains("scripts/freelife-webcam-setup.sh"), "{err}");
}
