//! The rotation of a HEIF/HEIC primary image (`irot`, `imir` properties).
//!
//! FFmpeg 7.1 exposes HEIF tile grids as a stream group but does not read `irot`, so the probe
//! reads it here from the `meta` box at the start of the file. Phone cameras store portrait
//! photos as a landscape grid plus `irot`.

use app_core::media::Rotation;

/// How much of the file head is read to find the `meta` box. Phones write it right after `ftyp`.
pub const HEAD_BYTES: usize = 256 * 1024;

/// A box: (fourcc, payload).
fn boxes(data: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut rest = data;
    std::iter::from_fn(move || {
        if rest.len() < 8 {
            return None;
        }
        let size = u32::from_be_bytes(rest[0..4].try_into().ok()?) as usize;
        let fourcc: [u8; 4] = rest[4..8].try_into().ok()?;
        let (header, size) = match size {
            1 if rest.len() >= 16 => (
                16,
                usize::try_from(u64::from_be_bytes(rest[8..16].try_into().ok()?)).ok()?,
            ),
            0 => (8, rest.len()),
            n => (8, n),
        };
        if size < header || size > rest.len() {
            return None;
        }
        let payload = &rest[header..size];
        rest = &rest[size..];
        Some((fourcc, payload))
    })
}

fn find<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(data).find(|(f, _)| f == fourcc).map(|(_, p)| p)
}

/// Clockwise quarter turns of the primary item, from its `irot` property; `None` if the file is
/// not HEIF or has no `irot` for the primary item. A mirror (`imir`) is ignored and logged.
pub fn primary_item_rotation(head: &[u8]) -> Option<Rotation> {
    let meta = find(head, b"meta")?;
    let meta = meta.get(4..)?; // full box: version + flags
    let pitm = find(meta, b"pitm")?;
    let primary = match pitm.first()? {
        0 => u32::from(u16::from_be_bytes(pitm.get(4..6)?.try_into().ok()?)),
        _ => u32::from_be_bytes(pitm.get(4..8)?.try_into().ok()?),
    };
    let iprp = find(meta, b"iprp")?;
    let properties: Vec<([u8; 4], &[u8])> = boxes(find(iprp, b"ipco")?).collect();
    let associated = associations(find(iprp, b"ipma")?, primary)?;

    let mut rotation = None;
    for index in associated {
        let Some((fourcc, payload)) = index.checked_sub(1).and_then(|i| properties.get(i)) else {
            continue;
        };
        match fourcc {
            b"irot" => {
                let ccw_quarters = payload.first()? & 3;
                rotation = Some(match ccw_quarters {
                    1 => Rotation::Cw270,
                    2 => Rotation::Cw180,
                    3 => Rotation::Cw90,
                    _ => Rotation::None,
                });
            }
            b"imir" => tracing::info!("HEIF image is mirrored; mirroring is ignored"),
            _ => {}
        }
    }
    rotation
}

/// The 1-based property indices associated with `item` in an `ipma` box, in order.
fn associations(ipma: &[u8], item: u32) -> Option<Vec<usize>> {
    let version = *ipma.first()?;
    let flags = u32::from_be_bytes([0, *ipma.get(1)?, *ipma.get(2)?, *ipma.get(3)?]);
    let mut pos = 4;
    let read = |pos: &mut usize, len: usize| -> Option<u32> {
        let bytes = ipma.get(*pos..*pos + len)?;
        *pos += len;
        Some(bytes.iter().fold(0, |acc, &b| (acc << 8) | u32::from(b)))
    };
    let count = read(&mut pos, 4)?;
    for _ in 0..count {
        let id = read(&mut pos, if version < 1 { 2 } else { 4 })?;
        let n = read(&mut pos, 1)?;
        let mut indices = Vec::with_capacity(n as usize);
        for _ in 0..n {
            let index = if flags & 1 != 0 {
                read(&mut pos, 2)? & 0x7fff
            } else {
                read(&mut pos, 1)? & 0x7f
            };
            indices.push(index as usize);
        }
        if id == item {
            return Some(indices);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = ((8 + payload.len()) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(fourcc);
        out.extend_from_slice(payload);
        out
    }

    fn heif(irot: Option<u8>, primary: u16) -> Vec<u8> {
        let mut ipco = bx(b"ispe", &[0; 12]);
        if let Some(angle) = irot {
            ipco.extend(bx(b"irot", &[angle]));
        }
        // item 1 -> properties 1 (ispe), 2 (irot, essential)
        let ipma = [&[0u8, 0, 0, 0, 0, 0, 0, 1, 0, 1, 2, 0x01, 0x82][..]].concat();
        let iprp = bx(b"iprp", &[bx(b"ipco", &ipco), bx(b"ipma", &ipma)].concat());
        let pitm = bx(
            b"pitm",
            &[&[0u8, 0, 0, 0][..], &primary.to_be_bytes()].concat(),
        );
        let meta = bx(b"meta", &[&[0u8; 4][..], &pitm, &iprp].concat());
        [bx(b"ftyp", b"heic\0\0\0\0mif1"), meta].concat()
    }

    #[test]
    fn reads_irot_of_the_primary_item() {
        assert_eq!(
            primary_item_rotation(&heif(Some(1), 1)),
            Some(Rotation::Cw270)
        );
        assert_eq!(
            primary_item_rotation(&heif(Some(3), 1)),
            Some(Rotation::Cw90)
        );
        assert_eq!(
            primary_item_rotation(&heif(Some(2), 1)),
            Some(Rotation::Cw180)
        );
        assert_eq!(
            primary_item_rotation(&heif(Some(0), 1)),
            Some(Rotation::None)
        );
    }

    #[test]
    fn no_irot_or_not_heif_is_none() {
        assert_eq!(primary_item_rotation(&heif(None, 1)), None);
        // The irot belongs to another item.
        assert_eq!(primary_item_rotation(&heif(Some(1), 7)), None);
        assert_eq!(primary_item_rotation(b"not a box at all"), None);
        assert_eq!(primary_item_rotation(&[]), None);
        // Truncated boxes are not read past their end.
        let file = heif(Some(1), 1);
        assert_eq!(primary_item_rotation(&file[..file.len() - 3]), None);
    }
}
