use std::{
    io::{ErrorKind, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context};

pub const SERIAL_BAUD: u32 = 3_000_000;
pub const MAX_SERIAL_JPEG: usize = u16::MAX as usize;
pub const MAX_MJPEG_FRAME: usize = 2 * 1024 * 1024;
const SERIAL_HEADER: [u8; 4] = [0xFF, 0xA0, 0xFF, 0xA1];
const SOI: [u8; 2] = [0xFF, 0xD8];
const EOI: [u8; 2] = [0xFF, 0xD9];
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const READ_TIMEOUT: Duration = Duration::from_secs(3);
const FRAME_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_HTTP_HEADER: usize = 16 * 1024;
const MAX_CHUNK_LINE: usize = 128;

pub enum RawFrame {
    Jpeg(Vec<u8>),
    Yuyv {
        width: usize,
        height: usize,
        stride: usize,
        data: Vec<u8>,
    },
}

pub trait FrameSource: Send {
    fn next_frame(&mut self) -> anyhow::Result<RawFrame>;
    fn describe(&self) -> String;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceSpec {
    Serial {
        path: String,
        baud: u32,
    },
    Http {
        host: String,
        port: u16,
        path: String,
    },
    V4l2 {
        path: String,
    },
}

impl SourceSpec {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let text = text.trim();
        if text.is_empty() {
            bail!("empty camera source");
        }
        let lower = text.to_ascii_lowercase();
        if let Some(rest) = strip_prefix_ci(text, "http://") {
            return parse_http(rest);
        }
        if lower.starts_with("https://") {
            bail!("{text}: https is not supported by camera firmware; use http://");
        }
        if let Some(rest) = strip_prefix_ci(text, "v4l2:") {
            return Ok(Self::V4l2 { path: rest.into() });
        }
        if let Some(rest) = strip_prefix_ci(text, "serial:") {
            return parse_serial(rest);
        }
        if text.starts_with("/dev/video") || text.starts_with("/dev/v4l/") {
            return Ok(Self::V4l2 { path: text.into() });
        }
        if text.starts_with('/')
            || lower.starts_with("com") && text[3..].chars().all(|c| c.is_ascii_digit() || c == '@')
        {
            return parse_serial(text);
        }
        if !text.contains('/')
            || text
                .split('/')
                .next()
                .is_some_and(|host| host.contains('.'))
        {
            return parse_http(text);
        }
        bail!("{text}: expected a serial port (/dev/ttyUSB0), an MJPEG URL (http://host/) or a V4L2 device (/dev/video0)")
    }
}

impl std::fmt::Display for SourceSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Serial { path, baud } => write!(f, "serial {path} @ {baud} baud"),
            Self::Http { host, port, path } => write!(f, "MJPEG http://{host}:{port}{path}"),
            Self::V4l2 { path } => write!(f, "V4L2 {path}"),
        }
    }
}

fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

fn parse_serial(text: &str) -> anyhow::Result<SourceSpec> {
    let (path, baud) = match text.rsplit_once('@') {
        Some((path, baud)) => (
            path,
            baud.parse()
                .map_err(|_| anyhow!("{text}: invalid baud rate {baud}"))?,
        ),
        None => (text, SERIAL_BAUD),
    };
    if path.is_empty() {
        bail!("{text}: missing serial port path");
    }
    Ok(SourceSpec::Serial {
        path: path.into(),
        baud,
    })
}

fn parse_http(rest: &str) -> anyhow::Result<SourceSpec> {
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
        let (host, tail) = inner
            .split_once(']')
            .ok_or_else(|| anyhow!("{rest}: unterminated IPv6 address"))?;
        let port = match tail.strip_prefix(':') {
            Some(port) => port.parse().map_err(|_| anyhow!("{rest}: invalid port"))?,
            None => 80,
        };
        (host.to_string(), port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (
                host.to_string(),
                port.parse()
                    .map_err(|_| anyhow!("{rest}: invalid port {port}"))?,
            ),
            None => (authority.to_string(), 80),
        }
    };
    if host.is_empty() {
        bail!("http://{rest}: missing host");
    }
    Ok(SourceSpec::Http {
        host,
        port,
        path: path.into(),
    })
}

pub fn open(spec: &SourceSpec) -> anyhow::Result<Box<dyn FrameSource>> {
    match spec {
        SourceSpec::Serial { path, baud } => Ok(Box::new(SerialSource::open(path, *baud)?)),
        SourceSpec::Http { host, port, path } => {
            Ok(Box::new(MjpegSource::open(host, *port, path)?))
        }
        SourceSpec::V4l2 { path } => Ok(Box::new(v4l2::V4l2Source::open(path)?)),
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

pub struct SerialParser {
    buffer: Vec<u8>,
    max_len: usize,
    pub rejected: u64,
}

impl SerialParser {
    pub fn new(max_len: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(max_len.min(MAX_SERIAL_JPEG) + 64),
            max_len: max_len.min(MAX_SERIAL_JPEG),
            rejected: 0,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
        let limit = 2 * (self.max_len + SERIAL_HEADER.len() + 2);
        if self.buffer.len() > limit {
            let excess = self.buffer.len() - limit;
            self.buffer.drain(..excess);
        }
    }

    pub fn next_frame(&mut self) -> Option<Vec<u8>> {
        loop {
            let Some(start) = find(&self.buffer, &SERIAL_HEADER) else {
                let keep = self.buffer.len().min(SERIAL_HEADER.len() - 1);
                let drop = self.buffer.len() - keep;
                self.buffer.drain(..drop);
                return None;
            };
            self.buffer.drain(..start);
            if self.buffer.len() < 6 {
                return None;
            }
            let len = u16::from_le_bytes([self.buffer[4], self.buffer[5]]) as usize;
            if len < 4 || len > self.max_len {
                self.rejected += 1;
                self.buffer.drain(..SERIAL_HEADER.len());
                continue;
            }
            if self.buffer.len() < 6 + len {
                return None;
            }
            let jpeg = &self.buffer[6..6 + len];
            if jpeg.starts_with(&SOI) && jpeg.ends_with(&EOI) {
                let frame = jpeg.to_vec();
                self.buffer.drain(..6 + len);
                return Some(frame);
            }
            self.rejected += 1;
            self.buffer.drain(..SERIAL_HEADER.len());
        }
    }
}

pub struct JpegScanner {
    buffer: Vec<u8>,
    in_frame: bool,
    scanned: usize,
    max_len: usize,
    pub overflows: u64,
}

impl JpegScanner {
    pub fn new(max_len: usize) -> Self {
        Self {
            buffer: Vec::new(),
            in_frame: false,
            scanned: 0,
            max_len,
            overflows: 0,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    pub fn next_frame(&mut self) -> Option<Vec<u8>> {
        loop {
            if !self.in_frame {
                match find(&self.buffer, &SOI) {
                    Some(start) => {
                        self.buffer.drain(..start);
                        self.in_frame = true;
                        self.scanned = SOI.len();
                    }
                    None => {
                        let keep = usize::from(self.buffer.last() == Some(&0xFF));
                        let drop = self.buffer.len() - keep;
                        self.buffer.drain(..drop);
                        return None;
                    }
                }
            }

            let from = self.scanned.saturating_sub(1).max(SOI.len());
            let mut marker = None;
            if from < self.buffer.len() {
                for (offset, pair) in self.buffer[from..].windows(2).enumerate() {
                    if pair[0] == 0xFF && (pair[1] == EOI[1] || pair[1] == SOI[1]) {
                        marker = Some((from + offset, pair[1]));
                        break;
                    }
                }
            }
            match marker {
                Some((index, byte)) if byte == EOI[1] => {
                    let frame: Vec<u8> = self.buffer.drain(..index + 2).collect();
                    self.in_frame = false;
                    self.scanned = 0;
                    if frame.len() <= self.max_len {
                        return Some(frame);
                    }
                    self.overflows += 1;
                }
                Some((index, _)) => {
                    self.buffer.drain(..index);
                    self.scanned = SOI.len();
                }
                None => {
                    self.scanned = self.buffer.len();
                    if self.buffer.len() > self.max_len {
                        self.overflows += 1;
                        self.buffer.clear();
                        self.in_frame = false;
                        self.scanned = 0;
                    }
                    return None;
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChunkState {
    Size,
    Data(usize),
    DataCr,
    DataLf,
    Done,
}

pub struct ChunkedDecoder {
    state: ChunkState,
    line: Vec<u8>,
}

impl Default for ChunkedDecoder {
    fn default() -> Self {
        Self {
            state: ChunkState::Size,
            line: Vec::new(),
        }
    }
}

impl ChunkedDecoder {
    pub fn is_done(&self) -> bool {
        self.state == ChunkState::Done
    }

    pub fn feed(&mut self, mut input: &[u8], out: &mut Vec<u8>) -> anyhow::Result<()> {
        while !input.is_empty() {
            match self.state {
                ChunkState::Size => {
                    let byte = input[0];
                    input = &input[1..];
                    if byte == b'\n' {
                        let text = String::from_utf8_lossy(&self.line).into_owned();
                        let text = text.trim_end_matches('\r');
                        let size = text.split(';').next().unwrap_or("").trim();
                        self.line.clear();
                        if size.is_empty() {
                            continue;
                        }
                        let size = usize::from_str_radix(size, 16)
                            .map_err(|_| anyhow!("invalid HTTP chunk size {size:?}"))?;
                        self.state = if size == 0 {
                            ChunkState::Done
                        } else {
                            ChunkState::Data(size)
                        };
                    } else {
                        if self.line.len() >= MAX_CHUNK_LINE {
                            bail!("HTTP chunk header too long");
                        }
                        self.line.push(byte);
                    }
                }
                ChunkState::Data(remaining) => {
                    let take = remaining.min(input.len());
                    out.extend_from_slice(&input[..take]);
                    input = &input[take..];
                    self.state = if take == remaining {
                        ChunkState::DataCr
                    } else {
                        ChunkState::Data(remaining - take)
                    };
                }
                ChunkState::DataCr => {
                    let byte = input[0];
                    input = &input[1..];
                    self.state = match byte {
                        b'\r' => ChunkState::DataLf,
                        b'\n' => ChunkState::Size,
                        _ => bail!("malformed HTTP chunk terminator"),
                    };
                }
                ChunkState::DataLf => {
                    let byte = input[0];
                    input = &input[1..];
                    if byte != b'\n' {
                        bail!("malformed HTTP chunk terminator");
                    }
                    self.state = ChunkState::Size;
                }
                ChunkState::Done => return Ok(()),
            }
        }
        Ok(())
    }
}

struct SerialSource {
    port: Box<dyn serialport::SerialPort>,
    path: String,
    baud: u32,
    parser: SerialParser,
    buffer: Vec<u8>,
}

impl SerialSource {
    fn open(path: &str, baud: u32) -> anyhow::Result<Self> {
        let port = serialport::new(path, baud)
            .timeout(Duration::from_millis(500))
            .open()
            .map_err(|e| serial_error(path, e))?;
        let _ = port.clear(serialport::ClearBuffer::Input);
        Ok(Self {
            port,
            path: path.into(),
            baud,
            parser: SerialParser::new(MAX_SERIAL_JPEG),
            buffer: vec![0; 16 * 1024],
        })
    }
}

fn serial_error(path: &str, error: serialport::Error) -> anyhow::Error {
    match error.kind() {
        serialport::ErrorKind::Io(ErrorKind::PermissionDenied) => anyhow!(
            "{path}: permission denied. Add your user to the serial group (dialout on Debian/Ubuntu/Fedora, uucp on Arch: sudo usermod -aG dialout $USER), then log out and back in"
        ),
        serialport::ErrorKind::Io(ErrorKind::NotFound) => {
            anyhow!("{path}: no such device (is the camera plugged in?)")
        }
        serialport::ErrorKind::NoDevice => {
            anyhow!("{path}: device unavailable or in use by another program ({error})")
        }
        _ => anyhow!("{path}: {error}"),
    }
}

impl FrameSource for SerialSource {
    fn next_frame(&mut self) -> anyhow::Result<RawFrame> {
        let deadline = Instant::now() + FRAME_TIMEOUT;
        loop {
            if let Some(frame) = self.parser.next_frame() {
                return Ok(RawFrame::Jpeg(frame));
            }
            if Instant::now() > deadline {
                bail!(
                    "{}: no camera frames for {}s (wrong port, baud rate or firmware?)",
                    self.path,
                    FRAME_TIMEOUT.as_secs()
                );
            }
            match self.port.read(&mut self.buffer) {
                Ok(0) => bail!("{}: serial port closed", self.path),
                Ok(n) => self.parser.push(&self.buffer[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::TimedOut | ErrorKind::WouldBlock | ErrorKind::Interrupted
                    ) => {}
                Err(e) => bail!("{}: {e}", self.path),
            }
        }
    }

    fn describe(&self) -> String {
        format!("serial {} @ {} baud", self.path, self.baud)
    }
}

struct MjpegSource {
    stream: TcpStream,
    url: String,
    chunked: Option<ChunkedDecoder>,
    scanner: JpegScanner,
    buffer: Vec<u8>,
    plain: Vec<u8>,
}

pub struct HttpHead {
    pub status: u16,
    pub chunked: bool,
    pub content_type: Option<String>,
    pub body_offset: usize,
}

pub fn parse_http_head(bytes: &[u8]) -> anyhow::Result<Option<HttpHead>> {
    let Some(end) = find(bytes, b"\r\n\r\n") else {
        if bytes.len() > MAX_HTTP_HEADER {
            bail!("HTTP response header too large");
        }
        return Ok(None);
    };
    let text = String::from_utf8_lossy(&bytes[..end]);
    let mut lines = text.split("\r\n");
    let status_line = lines.next().unwrap_or("");
    let mut parts = status_line.split_whitespace();
    let version = parts.next().unwrap_or("");
    if !version.starts_with("HTTP/") {
        bail!("not an HTTP response");
    }
    let status = parts
        .next()
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| anyhow!("malformed HTTP status line {status_line:?}"))?;
    let mut chunked = false;
    let mut content_type = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.trim().eq_ignore_ascii_case("transfer-encoding") {
            chunked = value.to_ascii_lowercase().contains("chunked");
        } else if name.trim().eq_ignore_ascii_case("content-type") {
            content_type = Some(value.to_string());
        }
    }
    Ok(Some(HttpHead {
        status,
        chunked,
        content_type,
        body_offset: end + 4,
    }))
}

impl MjpegSource {
    fn open(host: &str, port: u16, path: &str) -> anyhow::Result<Self> {
        let url = format!("http://{host}:{port}{path}");
        let address = (host, port)
            .to_socket_addrs()
            .with_context(|| format!("{url}: cannot resolve {host}"))?
            .next()
            .ok_or_else(|| anyhow!("{url}: {host} has no address"))?;
        let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)
            .with_context(|| format!("{url}: cannot connect to {address}"))?;
        stream.set_read_timeout(Some(READ_TIMEOUT))?;
        stream.set_write_timeout(Some(READ_TIMEOUT))?;
        let _ = stream.set_nodelay(true);
        let host_header = if host.contains(':') {
            format!("[{host}]")
        } else {
            host.to_string()
        };
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: {host_header}\r\nUser-Agent: visage\r\nAccept: multipart/x-mixed-replace, image/jpeg, */*\r\nConnection: close\r\n\r\n"
        )
        .with_context(|| format!("{url}: cannot send request"))?;

        let mut head = Vec::new();
        let mut buffer = vec![0u8; 16 * 1024];
        let parsed = loop {
            let n = stream
                .read(&mut buffer)
                .with_context(|| format!("{url}: no HTTP response"))?;
            if n == 0 {
                bail!("{url}: connection closed before the HTTP response");
            }
            head.extend_from_slice(&buffer[..n]);
            if let Some(parsed) = parse_http_head(&head).with_context(|| url.clone())? {
                break parsed;
            }
        };
        match parsed.status {
            200 => {}
            401 | 403 => bail!(
                "{url}: HTTP {} (camera stream needs authentication)",
                parsed.status
            ),
            404 => bail!("{url}: HTTP 404 (wrong stream path?)"),
            status => bail!("{url}: HTTP {status}"),
        }
        if let Some(kind) = parsed.content_type.as_deref() {
            let kind = kind.to_ascii_lowercase();
            if !kind.contains("multipart") && !kind.contains("jpeg") {
                log::warn!("Camera: {url} sends {kind}, expected an MJPEG stream");
            }
        }

        let mut me = Self {
            stream,
            url,
            chunked: parsed.chunked.then(ChunkedDecoder::default),
            scanner: JpegScanner::new(MAX_MJPEG_FRAME),
            buffer,
            plain: Vec::new(),
        };
        let body = head[parsed.body_offset..].to_vec();
        me.accept(&body)?;
        Ok(me)
    }

    fn accept(&mut self, bytes: &[u8]) -> anyhow::Result<()> {
        match self.chunked.as_mut() {
            Some(decoder) => {
                self.plain.clear();
                decoder.feed(bytes, &mut self.plain)?;
                self.scanner.push(&self.plain);
            }
            None => self.scanner.push(bytes),
        }
        Ok(())
    }
}

impl FrameSource for MjpegSource {
    fn next_frame(&mut self) -> anyhow::Result<RawFrame> {
        let deadline = Instant::now() + FRAME_TIMEOUT;
        loop {
            if let Some(frame) = self.scanner.next_frame() {
                return Ok(RawFrame::Jpeg(frame));
            }
            if self.chunked.as_ref().is_some_and(ChunkedDecoder::is_done) {
                bail!("{}: stream ended", self.url);
            }
            if Instant::now() > deadline {
                bail!(
                    "{}: no JPEG frames for {}s",
                    self.url,
                    FRAME_TIMEOUT.as_secs()
                );
            }
            let n = match self.stream.read(&mut self.buffer) {
                Ok(0) => bail!("{}: connection closed", self.url),
                Ok(n) => n,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    bail!("{}: timed out waiting for data", self.url)
                }
                Err(e) => bail!("{}: {e}", self.url),
            };
            let chunk = self.buffer[..n].to_vec();
            self.accept(&chunk)?;
        }
    }

    fn describe(&self) -> String {
        format!("MJPEG {}", self.url)
    }
}

mod v4l2 {
    use std::{io::ErrorKind, time::Duration};

    use anyhow::{anyhow, bail};
    use v4l::{
        buffer::Type, io::mmap::Stream, io::traits::CaptureStream, video::Capture, Device, FourCC,
    };

    use super::{FrameSource, RawFrame};

    pub struct V4l2Source {
        stream: Stream<'static>,
        _device: Device,
        path: String,
        width: usize,
        height: usize,
        stride: usize,
        mjpeg: bool,
    }

    impl V4l2Source {
        pub fn open(path: &str) -> anyhow::Result<Self> {
            let device = Device::with_path(path).map_err(|e| match e.kind() {
                ErrorKind::PermissionDenied => anyhow!(
                    "{path}: permission denied. Add your user to the video group (sudo usermod -aG video $USER), then log out and back in"
                ),
                ErrorKind::NotFound => anyhow!("{path}: no such device (is the camera plugged in?)"),
                _ => anyhow!("{path}: {e}"),
            })?;
            let current = device
                .format()
                .map_err(|e| anyhow!("{path}: not a video capture device ({e})"))?;
            let mut chosen = None;
            for fourcc in [b"MJPG", b"YUYV"] {
                let mut wanted = current;
                wanted.fourcc = FourCC::new(fourcc);
                if let Ok(format) = device.set_format(&wanted) {
                    if format.fourcc == wanted.fourcc {
                        chosen = Some(format);
                        break;
                    }
                }
            }
            let Some(format) = chosen else {
                bail!(
                    "{path}: camera offers {} but only MJPEG and YUYV are supported",
                    current.fourcc
                );
            };
            let mut stream = Stream::with_buffers(&device, Type::VideoCapture, 4)
                .map_err(|e| anyhow!("{path}: cannot start streaming ({e})"))?;
            stream.set_timeout(Duration::from_secs(3));
            let width = format.width as usize;
            let height = format.height as usize;
            let stride = (format.stride as usize).max(2 * width);
            Ok(Self {
                stream,
                _device: device,
                path: path.into(),
                width,
                height,
                stride,
                mjpeg: format.fourcc == FourCC::new(b"MJPG"),
            })
        }
    }

    impl FrameSource for V4l2Source {
        fn next_frame(&mut self) -> anyhow::Result<RawFrame> {
            let (buffer, meta) = self
                .stream
                .next()
                .map_err(|e| anyhow!("{}: {e}", self.path))?;
            let used = (meta.bytesused as usize).min(buffer.len());
            let data = buffer[..used].to_vec();
            if self.mjpeg {
                Ok(RawFrame::Jpeg(data))
            } else {
                Ok(RawFrame::Yuyv {
                    width: self.width,
                    height: self.height,
                    stride: self.stride,
                    data,
                })
            }
        }

        fn describe(&self) -> String {
            format!(
                "V4L2 {} {}x{} {}",
                self.path,
                self.width,
                self.height,
                if self.mjpeg { "MJPEG" } else { "YUYV" }
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(body: &[u8]) -> Vec<u8> {
        let mut bytes = SOI.to_vec();
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(&EOI);
        bytes
    }

    fn serial_frame(jpeg: &[u8]) -> Vec<u8> {
        let mut bytes = SERIAL_HEADER.to_vec();
        bytes.extend_from_slice(&(jpeg.len() as u16).to_le_bytes());
        bytes.extend_from_slice(jpeg);
        bytes
    }

    #[test]
    fn serial_parses_a_valid_frame() {
        let image = jpeg(&[1, 2, 3, 4]);
        let mut parser = SerialParser::new(MAX_SERIAL_JPEG);
        parser.push(&serial_frame(&image));
        assert_eq!(parser.next_frame(), Some(image));
        assert_eq!(parser.next_frame(), None);
    }

    #[test]
    fn serial_resyncs_after_garbage_and_split_reads() {
        let first = jpeg(&[9; 50]);
        let second = jpeg(&[7; 20]);
        let mut stream = vec![0x00, 0xFF, 0xA0, 0x13, 0xFF, 0xFF];
        stream.extend(serial_frame(&first));
        stream.extend([0xAA; 11]);
        stream.extend(serial_frame(&second));
        let mut parser = SerialParser::new(MAX_SERIAL_JPEG);
        let mut frames = Vec::new();
        for piece in stream.chunks(3) {
            parser.push(piece);
            while let Some(frame) = parser.next_frame() {
                frames.push(frame);
            }
        }
        assert_eq!(frames, vec![first, second]);
    }

    #[test]
    fn serial_waits_on_a_truncated_frame_then_recovers() {
        let good = jpeg(&[5; 30]);
        let mut truncated = serial_frame(&jpeg(&[1; 40]));
        truncated.truncate(20);
        let mut parser = SerialParser::new(MAX_SERIAL_JPEG);
        parser.push(&truncated);
        assert_eq!(parser.next_frame(), None);
        parser.push(&serial_frame(&good));
        parser.push(&serial_frame(&good));
        let mut frames = Vec::new();
        while let Some(frame) = parser.next_frame() {
            frames.push(frame);
        }
        assert!(!frames.is_empty());
        assert!(frames.iter().all(|f| *f == good));
    }

    #[test]
    fn serial_rejects_oversized_length() {
        let good = jpeg(&[3; 10]);
        let mut stream = SERIAL_HEADER.to_vec();
        stream.extend(2000u16.to_le_bytes());
        stream.extend([0u8; 16]);
        stream.extend(serial_frame(&good));
        let mut parser = SerialParser::new(1000);
        parser.push(&stream);
        assert_eq!(parser.next_frame(), Some(good));
        assert_eq!(parser.rejected, 1);
    }

    #[test]
    fn serial_rejects_bad_eoi() {
        let mut bad = jpeg(&[4; 12]);
        let last = bad.len() - 1;
        bad[last] = 0x00;
        let good = jpeg(&[8; 12]);
        let mut parser = SerialParser::new(MAX_SERIAL_JPEG);
        parser.push(&serial_frame(&bad));
        parser.push(&serial_frame(&good));
        assert_eq!(parser.next_frame(), Some(good));
        assert_eq!(parser.rejected, 1);
    }

    #[test]
    fn serial_rejects_missing_soi() {
        let mut parser = SerialParser::new(MAX_SERIAL_JPEG);
        parser.push(&serial_frame(&[0, 1, 2, 0xFF, 0xD9]));
        assert_eq!(parser.next_frame(), None);
        assert_eq!(parser.rejected, 1);
    }

    #[test]
    fn mjpeg_scanner_finds_frames_across_chunk_boundaries() {
        let first = jpeg(&[0x10, 0xFF, 0x00, 0x20]);
        let second = jpeg(&[0x30; 9]);
        let mut stream = b"--frame\r\nContent-Type: image/jpeg\r\n\r\n".to_vec();
        stream.extend(&first);
        stream.extend(b"\r\n--frame\r\nContent-Type: image/jpeg\r\n\r\n");
        stream.extend(&second);
        stream.extend(b"\r\n");
        for size in 1..8 {
            let mut scanner = JpegScanner::new(MAX_MJPEG_FRAME);
            let mut frames = Vec::new();
            for piece in stream.chunks(size) {
                scanner.push(piece);
                while let Some(frame) = scanner.next_frame() {
                    frames.push(frame);
                }
            }
            assert_eq!(
                frames,
                vec![first.clone(), second.clone()],
                "chunk size {size}"
            );
        }
    }

    #[test]
    fn mjpeg_scanner_restarts_on_a_new_soi_and_caps_size() {
        let good = jpeg(&[1; 4]);
        let mut stream = SOI.to_vec();
        stream.extend([2; 6]);
        stream.extend(&good);
        let mut scanner = JpegScanner::new(MAX_MJPEG_FRAME);
        scanner.push(&stream);
        assert_eq!(scanner.next_frame(), Some(good.clone()));

        let mut scanner = JpegScanner::new(16);
        scanner.push(&jpeg(&[5; 40]));
        assert_eq!(scanner.next_frame(), None);
        assert_eq!(scanner.overflows, 1);
        scanner.push(&good);
        assert_eq!(scanner.next_frame(), Some(good));
    }

    #[test]
    fn chunked_decoder_reassembles_split_chunks() {
        let body = b"5\r\nhello\r\n7;ext=1\r\n, world\r\n0\r\n\r\n";
        for size in 1..6 {
            let mut decoder = ChunkedDecoder::default();
            let mut out = Vec::new();
            for piece in body.chunks(size) {
                decoder.feed(piece, &mut out).unwrap();
            }
            assert_eq!(out, b"hello, world");
            assert!(decoder.is_done());
        }
        let mut decoder = ChunkedDecoder::default();
        assert!(decoder.feed(b"zz\r\n", &mut Vec::new()).is_err());
    }

    #[test]
    fn http_head_reports_status_and_chunking() {
        let head = b"HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace;boundary=frame\r\nTransfer-Encoding: chunked\r\n\r\nrest";
        let parsed = parse_http_head(head).unwrap().unwrap();
        assert_eq!(parsed.status, 200);
        assert!(parsed.chunked);
        assert_eq!(&head[parsed.body_offset..], b"rest");
        assert!(parse_http_head(b"HTTP/1.1 200 OK\r\n").unwrap().is_none());
        assert!(parse_http_head(b"garbage\r\n\r\n").is_err());
    }

    fn serve_once(response: Vec<u8>) -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut client, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = client.read(&mut request);
            for piece in response.chunks(7) {
                if client.write_all(piece).is_err() {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        });
        port
    }

    fn chunk(out: &mut Vec<u8>, data: &[u8]) {
        out.extend(format!("{:X}\r\n", data.len()).as_bytes());
        out.extend(data);
        out.extend(b"\r\n");
    }

    #[test]
    fn mjpeg_over_chunked_http_yields_decodable_frames() {
        let image = super::super::decode::TEST_JPEG.to_vec();
        let mut response =
            b"HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace;boundary=frame\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        for _ in 0..2 {
            chunk(
                &mut response,
                b"--frame\r\nContent-Type: image/jpeg\r\n\r\n",
            );
            chunk(&mut response, &image[..100]);
            chunk(&mut response, &image[100..]);
            chunk(&mut response, b"\r\n");
        }
        let port = serve_once(response);
        let spec = SourceSpec::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
        let mut source = open(&spec).unwrap();
        for _ in 0..2 {
            let RawFrame::Jpeg(frame) = source.next_frame().unwrap() else {
                panic!("expected a JPEG frame");
            };
            assert_eq!(frame, image);
            let decoded = super::super::decode::decode_jpeg(&frame, false).unwrap();
            assert_eq!((decoded.width, decoded.height), (8, 8));
        }
        assert!(source.next_frame().is_err());
    }

    #[test]
    fn http_errors_are_reported() {
        let port = serve_once(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec());
        let spec = SourceSpec::parse(&format!("http://127.0.0.1:{port}/stream")).unwrap();
        let error = open(&spec).err().unwrap().to_string();
        assert!(error.contains("404"), "{error}");
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        let spec = SourceSpec::parse(&format!("http://127.0.0.1:{port}/")).unwrap();
        assert!(open(&spec).is_err());
        let missing = SourceSpec::parse("/dev/visage-no-such-tty").unwrap();
        let error = open(&missing).err().unwrap().to_string();
        assert!(
            error.contains("no such device") || error.contains("visage-no-such-tty"),
            "{error}"
        );
    }

    #[test]
    fn sources_parse_by_shape() {
        assert_eq!(
            SourceSpec::parse("/dev/ttyUSB0").unwrap(),
            SourceSpec::Serial {
                path: "/dev/ttyUSB0".into(),
                baud: SERIAL_BAUD
            }
        );
        assert_eq!(
            SourceSpec::parse("/dev/ttyACM1@115200").unwrap(),
            SourceSpec::Serial {
                path: "/dev/ttyACM1".into(),
                baud: 115200
            }
        );
        assert_eq!(
            SourceSpec::parse("/dev/video2").unwrap(),
            SourceSpec::V4l2 {
                path: "/dev/video2".into()
            }
        );
        assert_eq!(
            SourceSpec::parse("http://etvr-left.local/").unwrap(),
            SourceSpec::Http {
                host: "etvr-left.local".into(),
                port: 80,
                path: "/".into()
            }
        );
        assert_eq!(
            SourceSpec::parse("HTTP://10.0.0.5:81/stream").unwrap(),
            SourceSpec::Http {
                host: "10.0.0.5".into(),
                port: 81,
                path: "/stream".into()
            }
        );
        assert_eq!(
            SourceSpec::parse("babble.local").unwrap(),
            SourceSpec::Http {
                host: "babble.local".into(),
                port: 80,
                path: "/".into()
            }
        );
        assert!(SourceSpec::parse("").is_err());
        assert!(SourceSpec::parse("https://x/").is_err());
        assert!(SourceSpec::parse("http://:80/").is_err());
    }
}
