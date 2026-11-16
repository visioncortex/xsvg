//! Minimal dependency-free PNG encoder (8-bit RGB, stored-DEFLATE).
//!
//! The images this crate emits are a handful of texels — a bilinear patch is a
//! 2×2, a grid field (gx+1)×(gy+1) — so compression is irrelevant; correctness
//! and zero dependencies are the point. The zlib stream uses stored (BTYPE=00)
//! blocks with the standard adler32, wrapped in IHDR/IDAT/IEND with CRC-32.
//!
//! A texture resampled to display resolution is megapixels, not texels, and
//! there compression decides the output size. [`encode_png_with`] filters the
//! scanlines and hands them to a zlib compressor the caller supplies, so this
//! crate still carries no dependency of its own.

/// Encode an interleaved RGB8 image (`w*h*3` bytes) as a PNG file.
pub fn encode_rgb_png(w: u32, h: u32, rgb: &[u8]) -> Vec<u8> {
    encode_png(w, h, rgb, 3)
}

/// Encode an interleaved RGBA8 image (`w*h*4` bytes, straight alpha) as a PNG.
pub fn encode_rgba_png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    encode_png(w, h, rgba, 4)
}

/// Encode an interleaved 8-bit image (`channels` 3 = RGB, 4 = straight-alpha
/// RGBA) with adaptively filtered scanlines, compressed by `zlib`: a function
/// from the filtered bytes to a complete zlib stream (RFC 1950).
pub fn encode_png_with(
    w: u32,
    h: u32,
    px: &[u8],
    channels: usize,
    zlib: impl FnOnce(&[u8]) -> Vec<u8>,
) -> Vec<u8> {
    assert!(channels == 3 || channels == 4);
    assert_eq!(px.len(), w as usize * h as usize * channels);
    let mut out = header(w, h, channels);
    let idat = zlib(&filtered(w as usize, h as usize, px, channels));
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn encode_png(w: u32, h: u32, px: &[u8], ch: u32) -> Vec<u8> {
    assert_eq!(px.len(), (w * h * ch) as usize);
    let mut out = header(w, h, ch as usize);

    // raw scanlines with filter byte 0
    let stride = (w * ch) as usize;
    let mut raw = Vec::with_capacity((stride + 1) * h as usize);
    for row in 0..h as usize {
        raw.push(0);
        raw.extend_from_slice(&px[row * stride..(row + 1) * stride]);
    }
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

fn header(w: u32, h: u32, channels: usize) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    // 8-bit; color type 2 (RGB) or 6 (RGBA)
    ihdr.extend_from_slice(&[8, if channels == 4 { 6 } else { 2 }, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    out
}

/// Scanlines with the PNG filter (§9) chosen per row by the usual heuristic:
/// the one whose output, read as signed bytes, has the least total magnitude.
/// A smooth field leaves near-constant residuals, which is what DEFLATE needs.
fn filtered(w: usize, h: usize, px: &[u8], bpp: usize) -> Vec<u8> {
    let stride = w * bpp;
    let mut out = Vec::with_capacity((stride + 1) * h);
    let zero = vec![0u8; stride];
    let mut best = vec![0u8; stride];
    let mut trial = vec![0u8; stride];
    for row in 0..h {
        let cur = &px[row * stride..(row + 1) * stride];
        let up = if row == 0 {
            &zero[..]
        } else {
            &px[(row - 1) * stride..row * stride]
        };
        let (mut kind, mut least) = (0u8, u64::MAX);
        for filter in 0..5u8 {
            let mut cost = 0u64;
            for i in 0..stride {
                let a = if i >= bpp { cur[i - bpp] } else { 0 };
                let c = if i >= bpp { up[i - bpp] } else { 0 };
                let b = up[i];
                let predicted = match filter {
                    0 => 0,
                    1 => a,
                    2 => b,
                    3 => ((a as u16 + b as u16) / 2) as u8,
                    _ => paeth(a, b, c),
                };
                let v = cur[i].wrapping_sub(predicted);
                trial[i] = v;
                cost += (v as i8).unsigned_abs() as u64;
            }
            if cost < least {
                (kind, least) = (filter, cost);
                std::mem::swap(&mut best, &mut trial);
            }
        }
        out.push(kind);
        out.extend_from_slice(&best);
    }
    out
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (pa, pb, pc) = (
        (b as i16 - c as i16).abs(),
        (a as i16 - c as i16).abs(),
        (a as i16 + b as i16 - 2 * c as i16).abs(),
    );
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// zlib wrapper around stored (uncompressed) DEFLATE blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut z = Vec::with_capacity(data.len() + 16);
    z.extend_from_slice(&[0x78, 0x01]); // CMF/FLG: 32K window, no preset, check ok
    let mut rest = data;
    loop {
        let take = rest.len().min(65_535);
        let last = take == rest.len();
        z.push(u8::from(last));
        z.extend_from_slice(&(take as u16).to_le_bytes());
        z.extend_from_slice(&(!(take as u16)).to_le_bytes());
        z.extend_from_slice(&rest[..take]);
        if last {
            break;
        }
        rest = &rest[take..];
    }
    z.extend_from_slice(&adler32(data).to_be_bytes());
    z
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (!(crc & 1)).wrapping_add(1) & 0xEDB8_8320);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_ieee_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        // and the constant IEND chunk CRC every PNG in the world carries
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }

    #[test]
    fn adler32_matches_the_zlib_check_value() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn rgba_png_declares_color_type_six() {
        let rgba = [255u8, 0, 0, 128, 0, 255, 0, 255];
        let png = encode_rgba_png(2, 1, &rgba);
        assert_eq!(png[25], 6, "color type must be RGBA");
        let idat_len = u32::from_be_bytes(png[33..37].try_into().unwrap()) as usize;
        let z = &png[41..41 + idat_len];
        let raw = &z[7..z.len() - 4];
        assert_eq!(raw, &[0, 255, 0, 0, 128, 0, 255, 0, 255]);
    }

    /// Undo the scanline filters (PNG §9.2), as a decoder would.
    fn unfilter(raw: &[u8], w: usize, h: usize, bpp: usize) -> (Vec<u8>, Vec<u8>) {
        let stride = w * bpp;
        let (mut px, mut kinds) = (vec![0u8; stride * h], Vec::new());
        for row in 0..h {
            let line = &raw[row * (stride + 1)..(row + 1) * (stride + 1)];
            kinds.push(line[0]);
            for i in 0..stride {
                let a = if i >= bpp {
                    px[row * stride + i - bpp]
                } else {
                    0
                };
                let b = if row > 0 {
                    px[(row - 1) * stride + i]
                } else {
                    0
                };
                let c = if row > 0 && i >= bpp {
                    px[(row - 1) * stride + i - bpp]
                } else {
                    0
                };
                let predicted = match line[0] {
                    0 => 0,
                    1 => a,
                    2 => b,
                    3 => ((a as u16 + b as u16) / 2) as u8,
                    4 => paeth(a, b, c),
                    other => panic!("filter {other}"),
                };
                px[row * stride + i] = line[1 + i].wrapping_add(predicted);
            }
        }
        (px, kinds)
    }

    #[test]
    fn filtered_scanlines_decode_to_the_source_pixels() {
        for bpp in [3usize, 4] {
            let (w, h) = (37usize, 11usize);
            // a smooth ramp, a noisy band and a flat band: each favours a different filter
            let px: Vec<u8> = (0..w * h * bpp)
                .map(|i| match (i / (w * bpp)) % 3 {
                    0 => (i % (w * bpp) * 3 + i / (w * bpp)) as u8,
                    1 => (i.wrapping_mul(2_654_435_761) >> 5) as u8,
                    _ => 200,
                })
                .collect();
            let mut seen = Vec::new();
            let png = encode_png_with(w as u32, h as u32, &px, bpp, |raw| {
                seen = raw.to_vec();
                zlib_stored(raw)
            });
            assert_eq!(seen.len(), (w * bpp + 1) * h);
            let (decoded, kinds) = unfilter(&seen, w, h, bpp);
            assert_eq!(decoded, px);
            assert!(
                kinds.iter().any(|k| *k != 0),
                "ramps must be filtered: {kinds:?}"
            );
            assert_eq!(png[25], if bpp == 4 { 6 } else { 2 });
            assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
        }
    }

    #[test]
    fn filtering_flattens_a_smooth_field() {
        // a horizontal ramp: after filtering every row is one repeated residual
        let (w, h) = (64usize, 4usize);
        let px: Vec<u8> = (0..w * h * 3)
            .map(|i| (i % (w * 3) / 3 * 2) as u8)
            .collect();
        let raw = filtered(w, h, &px, 3);
        let distinct: std::collections::BTreeSet<u8> = raw.iter().copied().collect();
        assert!(distinct.len() <= 4, "{distinct:?}");
    }

    #[test]
    fn png_structure_is_wellformed() {
        let rgb = [255u8, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
        let png = encode_rgb_png(2, 2, &rgb);
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[16..20], 2u32.to_be_bytes()); // width
        assert_eq!(&png[20..24], 2u32.to_be_bytes()); // height
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
        // decode the stored zlib stream back out of IDAT and compare scanlines
        let idat_len = u32::from_be_bytes(png[33..37].try_into().unwrap()) as usize;
        assert_eq!(&png[37..41], b"IDAT");
        let z = &png[41..41 + idat_len];
        // skip 2-byte zlib header + 5-byte stored block header; trailing 4 = adler
        let raw = &z[7..z.len() - 4];
        assert_eq!(raw, &[0, 255, 0, 0, 0, 255, 0, 0, 0, 0, 255, 255, 255, 255]);
    }
}
