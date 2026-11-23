use std::{
    io::Read,
    net::{Ipv4Addr, SocketAddr, TcpStream},
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

pub const PANEL: usize = 400;
const PANEL_PIXELS: usize = PANEL * PANEL;
const CAMERA_COUNT: usize = 5;
const LEFT_FACE_CAMERA: u32 = 2;
const RIGHT_FACE_CAMERA: u32 = 3;
const HEADER_BYTES: usize = 64;
const MAGIC: &[u8; 8] = b"QPLIVE3\0";
const VERSION: u32 = 3;
const GRAY8: u32 = 1;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const READ_TIMEOUT: Duration = Duration::from_secs(3);
const RETRY_DELAY: Duration = Duration::from_secs(1);

pub struct Frame {
    pub left: Vec<u8>,
    pub right: Vec<u8>,
}

#[derive(Default)]
struct SlotState {
    frame: Option<Frame>,
    connected: bool,
    last_frame: Option<Instant>,
}

#[derive(Default)]
pub struct FrameSlot {
    state: Mutex<SlotState>,
    signal: Condvar,
}

impl FrameSlot {
    fn lock(&self) -> std::sync::MutexGuard<'_, SlotState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn take(&self, timeout: Duration) -> Option<Frame> {
        let state = self.lock();
        let (mut state, _) = self
            .signal
            .wait_timeout_while(state, timeout, |s| s.frame.is_none())
            .unwrap_or_else(|e| e.into_inner());
        state.frame.take()
    }

    pub fn connected(&self) -> bool {
        self.lock().connected
    }

    pub fn last_frame(&self) -> Option<Instant> {
        self.lock().last_frame
    }

    fn put(&self, frame: Frame) {
        let mut state = self.lock();
        state.frame = Some(frame);
        state.last_frame = Some(Instant::now());
        drop(state);
        self.signal.notify_one();
    }

    fn set_connected(&self, connected: bool) {
        let mut state = self.lock();
        state.connected = connected;
        if !connected {
            state.frame = None;
        }
    }
}

struct Header {
    width: usize,
    payload_bytes: usize,
    camera_mask: u32,
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn parse_header(bytes: &[u8; HEADER_BYTES]) -> anyhow::Result<Header> {
    if &bytes[..8] != MAGIC || read_u32(bytes, 8) != VERSION {
        anyhow::bail!("unexpected stream header");
    }
    if read_u32(bytes, 12) as usize != HEADER_BYTES {
        anyhow::bail!("unexpected stream header size");
    }
    let width = read_u32(bytes, 32) as usize;
    let height = read_u32(bytes, 36) as usize;
    let stride = read_u32(bytes, 40) as usize;
    let pixel_format = read_u32(bytes, 44);
    let payload_bytes = read_u32(bytes, 48) as usize;
    let camera_mask = read_u32(bytes, 52);

    let cameras = (camera_mask & ((1 << CAMERA_COUNT) - 1)).count_ones() as usize;
    if cameras == 0
        || width != cameras * PANEL
        || height != PANEL
        || stride != width
        || pixel_format != GRAY8
        || payload_bytes != width * height
    {
        anyhow::bail!("unsupported frame layout {width}x{height}, format {pixel_format}");
    }

    Ok(Header {
        width,
        payload_bytes,
        camera_mask,
    })
}

fn panel_index(camera_mask: u32, camera: u32) -> Option<usize> {
    if camera_mask & (1 << camera) == 0 {
        return None;
    }
    Some((camera_mask & ((1 << camera) - 1)).count_ones() as usize)
}

fn copy_panel(payload: &[u8], width: usize, index: usize) -> Vec<u8> {
    let mut panel = Vec::with_capacity(PANEL_PIXELS);
    let start = index * PANEL;
    for row in payload.chunks_exact(width) {
        panel.extend_from_slice(&row[start..start + PANEL]);
    }
    panel
}

fn extract_frame(header: &Header, payload: &[u8]) -> Option<Frame> {
    let left = panel_index(header.camera_mask, LEFT_FACE_CAMERA)?;
    let right = panel_index(header.camera_mask, RIGHT_FACE_CAMERA)?;
    Some(Frame {
        left: copy_panel(payload, header.width, left),
        right: copy_panel(payload, header.width, right),
    })
}

fn read_frames(
    stream: &mut TcpStream,
    slot: &FrameSlot,
    address: SocketAddr,
    streaming: &mut bool,
) -> anyhow::Result<()> {
    let mut header_bytes = [0u8; HEADER_BYTES];
    let mut payload = Vec::new();
    let mut warned = false;
    loop {
        stream.read_exact(&mut header_bytes)?;
        let header = parse_header(&header_bytes)?;
        if !*streaming {
            *streaming = true;
            slot.set_connected(true);
            log::info!("Tongue: camera stream connected on {address}");
        }
        payload.resize(header.payload_bytes, 0);
        stream.read_exact(&mut payload)?;
        match extract_frame(&header, &payload) {
            Some(frame) => slot.put(frame),
            None if !warned => {
                log::warn!(
                    "Tongue: stream has no lower-face cameras (mask {:#x})",
                    header.camera_mask
                );
                warned = true;
            }
            None => {}
        }
    }
}

pub fn spawn(port: u16) -> Arc<FrameSlot> {
    let slot = Arc::new(FrameSlot::default());
    let worker_slot = slot.clone();
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));

    thread::Builder::new()
        .name("tongue-stream".into())
        .spawn(move || loop {
            if let Ok(mut stream) = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT) {
                let _ = stream.set_nodelay(true);
                let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
                let mut streaming = false;
                let result = read_frames(&mut stream, &worker_slot, address, &mut streaming);
                if streaming {
                    if let Err(e) = result {
                        log::warn!("Tongue: camera stream closed: {e}");
                    }
                    worker_slot.set_connected(false);
                } else if let Err(e) = result {
                    log::debug!("Tongue: camera port open but no stream: {e}");
                }
            }
            thread::sleep(RETRY_DELAY);
        })
        .expect("spawn tongue-stream thread");

    slot
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(width: u32, mask: u32) -> [u8; HEADER_BYTES] {
        let mut bytes = [0u8; HEADER_BYTES];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8..12].copy_from_slice(&VERSION.to_le_bytes());
        bytes[12..16].copy_from_slice(&(HEADER_BYTES as u32).to_le_bytes());
        bytes[32..36].copy_from_slice(&width.to_le_bytes());
        bytes[36..40].copy_from_slice(&(PANEL as u32).to_le_bytes());
        bytes[40..44].copy_from_slice(&width.to_le_bytes());
        bytes[44..48].copy_from_slice(&GRAY8.to_le_bytes());
        bytes[48..52].copy_from_slice(&(width * PANEL as u32).to_le_bytes());
        bytes[52..56].copy_from_slice(&mask.to_le_bytes());
        bytes
    }

    fn payload(cameras: &[u8]) -> Vec<u8> {
        let width = cameras.len() * PANEL;
        let mut bytes = vec![0u8; width * PANEL];
        for row in bytes.chunks_exact_mut(width) {
            for (index, camera) in cameras.iter().enumerate() {
                row[index * PANEL..(index + 1) * PANEL].fill(*camera);
            }
        }
        bytes
    }

    #[test]
    fn mouth_strip_maps_to_both_face_cameras() {
        let header = parse_header(&header(800, 0x0c)).unwrap();
        let frame = extract_frame(&header, &payload(&[2, 3])).unwrap();
        assert!(frame.left.iter().all(|&v| v == 2));
        assert!(frame.right.iter().all(|&v| v == 3));
        assert_eq!(frame.left.len(), PANEL_PIXELS);
    }

    #[test]
    fn full_strip_skips_eye_cameras() {
        let header = parse_header(&header(2000, 0x1f)).unwrap();
        let frame = extract_frame(&header, &payload(&[0, 1, 2, 3, 4])).unwrap();
        assert!(frame.left.iter().all(|&v| v == 2));
        assert!(frame.right.iter().all(|&v| v == 3));
    }

    #[test]
    fn eye_only_strip_has_no_frame() {
        let header = parse_header(&header(800, 0x03)).unwrap();
        assert!(extract_frame(&header, &payload(&[0, 1])).is_none());
    }

    #[test]
    fn mismatched_layout_is_rejected() {
        assert!(parse_header(&header(1200, 0x0c)).is_err());
        let mut bad_magic = header(800, 0x0c);
        bad_magic[0] = b'X';
        assert!(parse_header(&bad_magic).is_err());
    }
}
