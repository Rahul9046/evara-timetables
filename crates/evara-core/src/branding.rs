//! What counts as a usable logo, decided from the bytes themselves.
//!
//! # Why this is here and not behind an image crate
//!
//! A school logo is user-supplied binary data that the application will later embed in
//! reports and exports. Two things have to be true before it is stored, and neither is a
//! storage question or a presentation one:
//!
//! 1. it really is a PNG, a JPEG or a WebP — **decided by the content**, never by a
//!    filename, which the user may have renamed and which never crosses the IPC boundary
//!    anyway; and
//! 2. it is within the size and dimension limits the product is willing to carry inside a
//!    project file.
//!
//! Both are pure functions over `&[u8]`, so they live at the bottom of the stack where the
//! repository, a future exporter and the tests can all reach them.
//!
//! Pulling in a decoder for this would be the wrong trade. Everything below reads *header*
//! fields — a few dozen bytes at known offsets — and never decodes pixel data, so it cannot
//! be made to allocate a frame buffer or spend time proportional to the image. A full
//! decoder would do both, on untrusted input, to answer a question we are not asking.
//!
//! # What this deliberately does not do
//!
//! It does not verify that the whole file is well-formed, only that the header is coherent
//! and the declared dimensions are sane. A truncated PNG passes here and fails in the
//! viewer. That is the right split: the alternative is decoding every upload, and the
//! consequence of being wrong is a broken preview the user can replace, not corruption.

use serde::{Deserialize, Serialize};

/// The largest logo a project will carry: 2 MiB.
///
/// Generous for a logo and small enough that a project file stays comfortable to copy, and
/// that holding one in memory to hand over IPC is unremarkable. The column repeats this as
/// a `CHECK`, so it holds even against a write that bypasses this module.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

/// The largest edge accepted, in pixels.
///
/// A logo is shown at a few hundred pixels at most. Anything larger is a photograph that
/// has wandered into the wrong field, and storing it would cost a project file megabytes
/// to display a thumbnail.
pub const MAX_EDGE: u32 = 2048;

/// The smallest edge accepted, in pixels. Below this there is nothing to show.
pub const MIN_EDGE: u32 = 16;

/// An image format a school logo may be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum LogoFormat {
    /// PNG. The recommended one: it is the only form of the three with real transparency.
    Png,
    /// JPEG.
    Jpeg,
    /// WebP, in any of its lossy, lossless and extended forms.
    Webp,
}

impl LogoFormat {
    /// The IANA media type, which is also what the column stores and what a `data:` URL needs.
    #[must_use]
    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }

    /// Reads a stored media type back.
    ///
    /// Returns `None` for anything else rather than guessing, so a row written by a future
    /// build that supports more formats is reported as unreadable instead of mislabelled.
    #[must_use]
    pub fn from_media_type(value: &str) -> Option<Self> {
        match value {
            "image/png" => Some(Self::Png),
            "image/jpeg" => Some(Self::Jpeg),
            "image/webp" => Some(Self::Webp),
            _ => None,
        }
    }

    /// Every accepted media type, for a file picker's `accept` list.
    #[must_use]
    pub const fn all() -> [Self; 3] {
        [Self::Png, Self::Jpeg, Self::Webp]
    }
}

/// Why an uploaded image was refused.
///
/// Each variant carries what the user needs to act: which limit, and what their file
/// actually was. A bare "invalid image" would leave them guessing between four different
/// problems.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LogoError {
    /// No bytes at all.
    #[error("that file is empty")]
    Empty,
    /// The header matches no format this build accepts.
    #[error("that file is not a PNG, JPEG or WebP image")]
    UnsupportedFormat,
    /// Recognised as one of the three, but the header is cut short or self-contradictory.
    #[error("that image file is damaged, or not fully written")]
    Damaged,
    /// Past [`MAX_BYTES`].
    #[error("that image is {actual} bytes; the limit is {limit}")]
    TooLarge {
        /// The file's size.
        actual: usize,
        /// [`MAX_BYTES`].
        limit: usize,
    },
    /// An edge past [`MAX_EDGE`].
    #[error("that image is {width}x{height}; the largest accepted is {limit}x{limit}")]
    TooManyPixels {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// [`MAX_EDGE`].
        limit: u32,
    },
    /// An edge below [`MIN_EDGE`].
    #[error("that image is {width}x{height}; the smallest accepted is {limit}x{limit}")]
    TooFewPixels {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// [`MIN_EDGE`].
        limit: u32,
    },
}

/// What inspection learned about an image, without decoding it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct LogoImage {
    /// The format, as read from the file's own header.
    pub format: LogoFormat,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Size of the file in bytes.
    pub byte_size: u32,
}

impl LogoImage {
    /// Whether the image is square, which is what the interface recommends.
    ///
    /// Advice, never a rule: a school with a wide wordmark has a real logo and the product
    /// has no business refusing it.
    #[must_use]
    pub const fn is_square(&self) -> bool {
        self.width == self.height
    }

    /// Identifies an image and checks it against the limits.
    ///
    /// # Errors
    ///
    /// [`LogoError`], one variant per thing that can be wrong, each naming the limit it
    /// broke and the value that broke it.
    pub fn inspect(bytes: &[u8]) -> Result<Self, LogoError> {
        if bytes.is_empty() {
            return Err(LogoError::Empty);
        }
        if bytes.len() > MAX_BYTES {
            // Checked before the header so an enormous file is refused for being enormous,
            // which is the actionable complaint, rather than for whatever its header says.
            return Err(LogoError::TooLarge {
                actual: bytes.len(),
                limit: MAX_BYTES,
            });
        }

        let format = sniff(bytes).ok_or(LogoError::UnsupportedFormat)?;
        let (width, height) = match format {
            LogoFormat::Png => png_dimensions(bytes),
            LogoFormat::Jpeg => jpeg_dimensions(bytes),
            LogoFormat::Webp => webp_dimensions(bytes),
        }
        .ok_or(LogoError::Damaged)?;

        if width == 0 || height == 0 {
            return Err(LogoError::Damaged);
        }
        if width > MAX_EDGE || height > MAX_EDGE {
            return Err(LogoError::TooManyPixels {
                width,
                height,
                limit: MAX_EDGE,
            });
        }
        if width < MIN_EDGE || height < MIN_EDGE {
            return Err(LogoError::TooFewPixels {
                width,
                height,
                limit: MIN_EDGE,
            });
        }

        Ok(Self {
            format,
            width,
            height,
            // `bytes.len()` is bounded by MAX_BYTES above, so this cannot truncate.
            byte_size: u32::try_from(bytes.len()).unwrap_or(u32::MAX),
        })
    }
}

/// The format a file's own first bytes claim.
///
/// This is the whole of "validate contents rather than the extension": nothing upstream
/// passes a filename, and a `.png` holding a JPEG is identified as a JPEG.
#[must_use]
pub fn sniff(bytes: &[u8]) -> Option<LogoFormat> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some(LogoFormat::Png);
    }
    // Every JPEG begins with the Start of Image marker followed by another marker.
    if bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return Some(LogoFormat::Jpeg);
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some(LogoFormat::Webp);
    }
    None
}

/// Reads four big-endian bytes at `offset`.
fn be_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// Reads two big-endian bytes at `offset`.
fn be_u16(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 2)?;
    Some(u32::from(u16::from_be_bytes([slice[0], slice[1]])))
}

/// PNG: the IHDR chunk is mandatory and must come first.
///
/// Layout is fixed — 8-byte signature, 4-byte length, the type `IHDR`, then width and
/// height as big-endian `u32` — so there is nothing to scan for.
fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    Some((be_u32(bytes, 16)?, be_u32(bytes, 20)?))
}

/// JPEG: walk the segment list to the first Start of Frame.
///
/// A JPEG is a chain of `FF <marker> <length>` segments and the frame header can sit
/// behind any number of them, so this is the one format that needs a scan rather than a
/// fixed offset. The loop is bounded by the input and every step advances, so a malformed
/// file ends the walk rather than spinning.
fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2; // past the Start of Image marker.
    loop {
        // Fill bytes: any number of 0xFF may pad the gap before a marker.
        while bytes.get(at) == Some(&0xFF) && bytes.get(at + 1) == Some(&0xFF) {
            at += 1;
        }
        if bytes.get(at)? != &0xFF {
            return None;
        }
        let marker = *bytes.get(at + 1)?;
        at += 2;

        match marker {
            // Start of Frame, in every flavour that carries dimensions. C4 is a Huffman
            // table, C8 is reserved and CC is an arithmetic-coding table: all are ordinary
            // segments that happen to sit in the same numeric range.
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                // length(2) precision(1) height(2) width(2)
                return Some((be_u16(bytes, at + 5)?, be_u16(bytes, at + 3)?));
            }
            // Standalone markers: no length field follows, so there is nothing to skip.
            0xD0..=0xD9 | 0x01 => {}
            // Start of Scan means the entropy-coded image data begins and no frame header
            // was found. Scanning further would be reading pixel data as if it were
            // structure.
            0xDA => return None,
            _ => {
                let length = be_u16(bytes, at)? as usize;
                // A segment's length field includes its own two bytes, so anything below
                // two is nonsense and would make this loop stand still.
                if length < 2 {
                    return None;
                }
                at += length;
            }
        }
    }
}

/// WebP: three different chunk layouts, one per flavour.
///
/// `VP8 ` is lossy, `VP8L` lossless, and `VP8X` the extended form that carries a canvas
/// size and wraps one of the other two. All three keep their dimensions in the first few
/// bytes of the chunk, little-endian and bit-packed.
fn webp_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let chunk = bytes.get(12..16)?;
    match chunk {
        b"VP8 " => {
            // Frame tag(3), then the start code, then two 16-bit fields whose low 14 bits
            // are the dimensions.
            if bytes.get(23..26)? != [0x9D, 0x01, 0x2A] {
                return None;
            }
            let width = u32::from(u16::from_le_bytes([*bytes.get(26)?, *bytes.get(27)?]));
            let height = u32::from(u16::from_le_bytes([*bytes.get(28)?, *bytes.get(29)?]));
            Some((width & 0x3FFF, height & 0x3FFF))
        }
        b"VP8L" => {
            if bytes.get(20)? != &0x2F {
                return None;
            }
            let packed = u32::from_le_bytes([
                *bytes.get(21)?,
                *bytes.get(22)?,
                *bytes.get(23)?,
                *bytes.get(24)?,
            ]);
            // 14 bits each, stored one less than the true size.
            Some(((packed & 0x3FFF) + 1, ((packed >> 14) & 0x3FFF) + 1))
        }
        b"VP8X" => {
            // flags(4), then canvas width and height as 24-bit little-endian, each one
            // less than the true size.
            let read24 = |offset: usize| -> Option<u32> {
                let slice = bytes.get(offset..offset + 3)?;
                Some(u32::from_le_bytes([slice[0], slice[1], slice[2], 0]))
            };
            Some((read24(24)? + 1, read24(27)? + 1))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{LogoError, LogoFormat, LogoImage, MAX_BYTES, MAX_EDGE, MIN_EDGE, sniff};

    /// A PNG header with the given dimensions. Only the parts this module reads are real.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes
    }

    /// A JPEG with `leading` ordinary segments before its Start of Frame.
    fn jpeg(width: u32, height: u32, leading: usize) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xD8];
        for _ in 0..leading {
            // An APP0 segment: length 16, then fifteen bytes of payload.
            bytes.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x10]);
            bytes.extend_from_slice(&[0u8; 14]);
        }
        bytes.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08]);
        bytes.extend_from_slice(&u16::try_from(height).unwrap().to_be_bytes());
        bytes.extend_from_slice(&u16::try_from(width).unwrap().to_be_bytes());
        bytes.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        bytes
    }

    /// A RIFF container around one chunk.
    fn riff(chunk: &[u8], payload: &[u8]) -> Vec<u8> {
        let length = u32::try_from(payload.len()).expect("test payloads are small");
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&(length + 12).to_le_bytes());
        bytes.extend_from_slice(b"WEBP");
        bytes.extend_from_slice(chunk);
        bytes.extend_from_slice(&length.to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn webp_lossy(width: u16, height: u16) -> Vec<u8> {
        let mut payload = vec![0x00, 0x00, 0x00, 0x9D, 0x01, 0x2A];
        payload.extend_from_slice(&width.to_le_bytes());
        payload.extend_from_slice(&height.to_le_bytes());
        riff(b"VP8 ", &payload)
    }

    fn webp_lossless(width: u32, height: u32) -> Vec<u8> {
        let packed = (width - 1) | ((height - 1) << 14);
        let mut payload = vec![0x2F];
        payload.extend_from_slice(&packed.to_le_bytes());
        riff(b"VP8L", &payload)
    }

    fn webp_extended(width: u32, height: u32) -> Vec<u8> {
        let mut payload = vec![0, 0, 0, 0];
        payload.extend_from_slice(&(width - 1).to_le_bytes()[0..3]);
        payload.extend_from_slice(&(height - 1).to_le_bytes()[0..3]);
        riff(b"VP8X", &payload)
    }

    #[test]
    fn a_png_is_identified_and_measured() {
        let image = LogoImage::inspect(&png(512, 512)).expect("a real png");
        assert_eq!(image.format, LogoFormat::Png);
        assert_eq!((image.width, image.height), (512, 512));
        assert!(image.is_square());
    }

    #[test]
    fn a_jpeg_is_measured_however_many_segments_precede_its_frame() {
        for leading in [0, 1, 5, 40] {
            let image = LogoImage::inspect(&jpeg(800, 600, leading)).expect("a real jpeg");
            assert_eq!(image.format, LogoFormat::Jpeg);
            assert_eq!((image.width, image.height), (800, 600));
            assert!(!image.is_square());
        }
    }

    #[test]
    fn all_three_webp_flavours_are_measured() {
        for (bytes, label) in [
            (webp_lossy(320, 240), "lossy"),
            (webp_lossless(320, 240), "lossless"),
            (webp_extended(320, 240), "extended"),
        ] {
            let image = LogoImage::inspect(&bytes).unwrap_or_else(|e| panic!("{label}: {e}"));
            assert_eq!(image.format, LogoFormat::Webp, "{label}");
            assert_eq!((image.width, image.height), (320, 240), "{label}");
        }
    }

    #[test]
    fn the_format_comes_from_the_content_not_the_name() {
        // A JPEG that someone renamed to .png. Nothing here ever saw the name, and the
        // answer is JPEG.
        assert_eq!(sniff(&jpeg(64, 64, 0)), Some(LogoFormat::Jpeg));
        assert_eq!(sniff(&png(64, 64)), Some(LogoFormat::Png));
        assert_eq!(sniff(&webp_lossy(64, 64)), Some(LogoFormat::Webp));
    }

    #[test]
    fn something_that_is_not_an_image_is_refused() {
        for bytes in [
            b"GIF89a................".to_vec(),
            b"%PDF-1.7 ...............".to_vec(),
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>".to_vec(),
            b"MZ\x90\x00\x03\x00\x00\x00".to_vec(),
            vec![0u8; 64],
        ] {
            assert_eq!(
                LogoImage::inspect(&bytes),
                Err(LogoError::UnsupportedFormat),
                "{:?} was accepted",
                &bytes[..8.min(bytes.len())]
            );
        }
    }

    #[test]
    fn an_empty_file_says_so_rather_than_being_an_unknown_format() {
        assert_eq!(LogoImage::inspect(&[]), Err(LogoError::Empty));
    }

    #[test]
    fn a_truncated_header_is_damage_not_an_unknown_format() {
        // Recognisably a PNG, then nothing. "Not an image" would send the user looking for
        // a different file when what they have is a bad copy of the right one.
        let full = png(64, 64);
        for cut in [9, 12, 16, 20, 23] {
            assert_eq!(
                LogoImage::inspect(&full[..cut]),
                Err(LogoError::Damaged),
                "a {cut}-byte png was not reported as damaged"
            );
        }
    }

    #[test]
    fn a_jpeg_whose_scan_starts_before_any_frame_is_damage() {
        let bytes = vec![0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02];
        assert_eq!(LogoImage::inspect(&bytes), Err(LogoError::Damaged));
    }

    #[test]
    fn a_jpeg_with_a_nonsense_segment_length_terminates() {
        // Length 0 would leave the walk standing still. The test is that this returns at
        // all rather than what it returns.
        let bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(LogoImage::inspect(&bytes), Err(LogoError::Damaged));
    }

    #[test]
    fn a_webp_with_an_unknown_chunk_is_damage() {
        assert_eq!(
            LogoImage::inspect(&riff(b"ANIM", &[0u8; 16])),
            Err(LogoError::Damaged)
        );
    }

    #[test]
    fn an_oversized_file_is_refused_for_its_size() {
        let mut bytes = png(64, 64);
        bytes.resize(MAX_BYTES + 1, 0);
        assert_eq!(
            LogoImage::inspect(&bytes),
            Err(LogoError::TooLarge {
                actual: MAX_BYTES + 1,
                limit: MAX_BYTES,
            })
        );
    }

    #[test]
    fn a_file_exactly_at_the_limit_is_accepted() {
        let mut bytes = png(64, 64);
        bytes.resize(MAX_BYTES, 0);
        let image = LogoImage::inspect(&bytes).expect("exactly at the limit");
        assert_eq!(image.byte_size as usize, MAX_BYTES);
    }

    #[test]
    fn an_enormous_canvas_is_refused_for_its_pixels() {
        assert_eq!(
            LogoImage::inspect(&png(MAX_EDGE + 1, 64)),
            Err(LogoError::TooManyPixels {
                width: MAX_EDGE + 1,
                height: 64,
                limit: MAX_EDGE,
            })
        );
        // A header claiming a gigantic image is refused on its own say-so, without
        // allocating anything: this file is 33 bytes long.
        assert!(matches!(
            LogoImage::inspect(&png(60_000, 60_000)),
            Err(LogoError::TooManyPixels { .. })
        ));
    }

    #[test]
    fn a_tiny_canvas_is_refused_for_its_pixels() {
        assert_eq!(
            LogoImage::inspect(&png(8, 8)),
            Err(LogoError::TooFewPixels {
                width: 8,
                height: 8,
                limit: MIN_EDGE,
            })
        );
    }

    #[test]
    fn the_edges_of_the_dimension_range_are_accepted() {
        for edge in [MIN_EDGE, MAX_EDGE] {
            assert!(
                LogoImage::inspect(&png(edge, edge)).is_ok(),
                "{edge} should be accepted"
            );
        }
    }

    #[test]
    fn a_zero_dimension_is_damage_rather_than_too_few_pixels() {
        // Nothing legitimate produces this, so it says the file is broken rather than
        // inviting the user to find a bigger version of it.
        assert_eq!(LogoImage::inspect(&png(0, 64)), Err(LogoError::Damaged));
        assert_eq!(LogoImage::inspect(&png(64, 0)), Err(LogoError::Damaged));
    }

    #[test]
    fn media_types_round_trip() {
        for format in LogoFormat::all() {
            assert_eq!(
                LogoFormat::from_media_type(format.media_type()),
                Some(format)
            );
        }
        assert_eq!(LogoFormat::from_media_type("image/gif"), None);
        assert_eq!(LogoFormat::from_media_type(""), None);
    }

    #[test]
    fn inspection_never_reads_past_the_input() {
        // Every prefix of every sample, to prove the offsets are all bounds-checked. A
        // panic here is a crash on an untrusted upload.
        for sample in [
            png(64, 64),
            jpeg(64, 64, 3),
            webp_lossy(64, 64),
            webp_lossless(64, 64),
            webp_extended(64, 64),
        ] {
            for cut in 0..=sample.len() {
                let _ = LogoImage::inspect(&sample[..cut]);
            }
        }
    }
}
