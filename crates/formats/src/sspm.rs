//! Parser for Sound Space Plus `.sspm` map files (versions 1 and 2).
//!
//! Rhythia grew out of Sound Space Plus and serves `.sspm` files from its
//! own map API, so replays can reference them directly. Both versions share
//! the note model of the cache JSON: a time in ms plus (x, y) on the 0..2
//! grid, with off-grid "quantum" notes as floats. Layout follows the
//! community spec (basils-garden/types, sspm/v1.md and v2.md); the field
//! walk mirrors gerhaarrd/rhr2mp4's MIT-licensed reader. All values are
//! little-endian.
//!
//! v2 stores its payloads behind (offset, length) pointers: custom data,
//! audio, cover, marker definitions and markers. Notes are the markers
//! whose definition is named `ssp_note` carrying a single position value
//! (type 0x07: a quantum flag, then u8 or f32 coordinate pairs).

use crate::map::{Map, MapMeta, Note};
use crate::{Error, Result};

const SIGNATURE: &[u8; 4] = b"SS+m";

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.data.len())
            .ok_or_else(|| Error::Malformed(format!("sspm: EOF reading {n} bytes")))?;
        let v = &self.data[self.pos..end];
        self.pos = end;
        Ok(v)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    /// v2 length-prefixed UTF-8 string.
    fn string16(&mut self) -> Result<String> {
        let n = self.u16()? as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }

    /// v1 newline-terminated string.
    fn line(&mut self) -> Result<String> {
        let rest = &self.data[self.pos..];
        let end = rest
            .iter()
            .position(|&b| b == 0x0a)
            .ok_or_else(|| Error::Malformed("sspm: unterminated v1 string".into()))?;
        let v = String::from_utf8_lossy(&rest[..end]).into_owned();
        self.pos += end + 1;
        Ok(v)
    }

    /// Data type 0x07: quantum flag + (u8, u8) or (f32, f32).
    fn position(&mut self) -> Result<(f32, f32)> {
        if self.u8()? != 0 {
            Ok((self.f32()?, self.f32()?))
        } else {
            Ok((self.u8()? as f32, self.u8()? as f32))
        }
    }

    /// Consumes one marker value of the given data type.
    fn skip_value(&mut self, type_id: u8) -> Result<()> {
        match type_id {
            0x00 => {}
            0x01 => self.pos += 1,
            0x02 => self.pos += 2,
            0x03 | 0x05 => self.pos += 4,
            0x04 | 0x06 => self.pos += 8,
            0x07 => {
                self.position()?;
            }
            0x08 | 0x09 => {
                let n = self.u16()? as usize;
                self.take(n)?;
            }
            0x0a | 0x0b => {
                let n = self.u32()? as usize;
                self.take(n)?;
            }
            0x0c => {
                let sub = self.u8()?;
                let count = self.u16()?;
                for _ in 0..count {
                    self.skip_value(sub)?;
                }
            }
            other => {
                return Err(Error::Malformed(format!(
                    "sspm: unknown data type 0x{other:02x}"
                )))
            }
        }
        if self.pos > self.data.len() {
            return Err(Error::Malformed("sspm: value ran past EOF".into()));
        }
        Ok(())
    }
}

fn slice_at(data: &[u8], off: u64, len: u64) -> Result<&[u8]> {
    let (off, len) = (off as usize, len as usize);
    off.checked_add(len)
        .filter(|&e| e <= data.len())
        .map(|end| &data[off..end])
        .ok_or_else(|| Error::Malformed("sspm: pointer past EOF".into()))
}

fn parse_v1(r: &mut Reader) -> Result<Map> {
    let map_id = r.line()?;
    let map_name = r.line()?;
    let creators = r.line()?;
    let last_ms = r.u32()?;
    let note_count = r.u32()?;
    let difficulty = r.u8()?;

    let mut cover = None;
    let cover_type = r.u8()?;
    if cover_type != 0 {
        let n = r.u64()? as usize;
        let bytes = r.take(n)?;
        // 0x01 was a deprecated raw-pixel format; only PNG (0x02) is usable.
        if cover_type == 0x02 {
            cover = Some(bytes.to_vec());
        }
    }
    let mut audio = None;
    if r.u8()? != 0 {
        let n = r.u64()? as usize;
        audio = Some(r.take(n)?.to_vec());
    }

    // Bound the preallocation by what the file could actually hold.
    let cap = (note_count as usize).min(r.data.len().saturating_sub(r.pos) / 6);
    let mut notes = Vec::with_capacity(cap);
    for _ in 0..note_count {
        let ms = r.u32()?;
        let (x, y) = r.position()?;
        notes.push(Note {
            time_ms: ms as i64,
            x,
            y,
        });
    }
    notes.sort_by_key(|n| n.time_ms);

    let mappers: Vec<String> = creators
        .replace(" & ", ", ")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    Ok(Map {
        meta: MapMeta {
            legacy_id: map_id,
            song_name: map_name.clone(),
            title: map_name,
            mappers,
            duration_ms: last_ms as i64,
            difficulty: difficulty as i64,
            ..MapMeta::default()
        },
        notes,
        audio,
        cover,
    })
}

fn parse_v2(data: &[u8], r: &mut Reader) -> Result<Map> {
    r.take(20)?; // sha1 of the marker data (often zeroed; unverified)
    let last_ms = r.u32()?;
    r.u32()?; // note count (recomputed from the markers)
    r.u32()?; // total marker count
    let difficulty = r.u8()?;
    let star_rating = r.u16()? as f64;
    r.take(3)?; // has audio / has cover / requires mod flags

    let mut ptr = [(0u64, 0u64); 5];
    for p in &mut ptr {
        *p = (r.u64()?, r.u64()?);
    }
    let [(custom_off, custom_len), (audio_off, audio_len), (cover_off, cover_len), (defs_off, defs_len), (markers_off, markers_len)] =
        ptr;

    let map_id = r.string16()?;
    let map_name = r.string16()?;
    let song_name = r.string16()?;
    let mapper_count = r.u16()?;
    let mut mappers = Vec::with_capacity(mapper_count.min(64) as usize);
    for _ in 0..mapper_count {
        mappers.push(r.string16()?);
    }

    let mut custom_difficulty_name = String::new();
    if custom_len > 0 {
        let mut c = Reader::new(slice_at(data, custom_off, custom_len)?);
        let fields = c.u16()?;
        for _ in 0..fields {
            let name = c.string16()?;
            let type_id = c.u8()?;
            if name == "difficulty_name" && type_id == 0x09 {
                custom_difficulty_name = c.string16()?;
            } else {
                c.skip_value(type_id)?;
            }
        }
    }

    // Marker definitions: which value types each marker kind carries.
    let mut d = Reader::new(slice_at(data, defs_off, defs_len)?);
    let mut definitions: Vec<(String, Vec<u8>)> = Vec::new();
    let def_count = d.u8()?;
    for _ in 0..def_count {
        let name = d.string16()?;
        let value_count = d.u8()?;
        let types: Vec<u8> = d.take(value_count as usize)?.to_vec();
        if d.u8()? != 0 {
            return Err(Error::Malformed("sspm: malformed marker definition".into()));
        }
        definitions.push((name, types));
    }

    let mut m = Reader::new(slice_at(data, markers_off, markers_len)?);
    let mut notes = Vec::new();
    while m.pos < m.data.len() {
        let ms = m.u32()?;
        let def_index = m.u8()? as usize;
        let (name, types) = definitions.get(def_index).ok_or_else(|| {
            Error::Malformed(format!("sspm: marker references definition {def_index}"))
        })?;
        if name == "ssp_note" && types.as_slice() == [0x07] {
            let (x, y) = m.position()?;
            notes.push(Note {
                time_ms: ms as i64,
                x,
                y,
            });
        } else {
            for &t in types {
                m.skip_value(t)?;
            }
        }
    }
    notes.sort_by_key(|n| n.time_ms);

    let audio = (audio_len > 0)
        .then(|| slice_at(data, audio_off, audio_len).map(<[u8]>::to_vec))
        .transpose()?;
    let cover = (cover_len > 0)
        .then(|| slice_at(data, cover_off, cover_len).map(<[u8]>::to_vec))
        .transpose()?;

    Ok(Map {
        meta: MapMeta {
            legacy_id: map_id,
            song_name: if song_name.is_empty() {
                map_name.clone()
            } else {
                song_name
            },
            title: map_name,
            mappers,
            duration_ms: last_ms as i64,
            difficulty: difficulty as i64,
            custom_difficulty_name,
            star_rating,
            ..MapMeta::default()
        },
        notes,
        audio,
        cover,
    })
}

/// Parses `.sspm` bytes (v1 or v2) into the shared [`Map`] model.
pub fn parse(data: &[u8]) -> Result<Map> {
    let mut r = Reader::new(data);
    if r.take(4)? != SIGNATURE {
        return Err(Error::Malformed("sspm: bad signature".into()));
    }
    match r.u16()? {
        1 => {
            r.take(2)?; // reserved
            parse_v1(&mut r)
        }
        2 => {
            r.take(4)?; // reserved
            parse_v2(data, &mut r)
        }
        v => Err(Error::Malformed(format!("sspm: unsupported version {v}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal v2 file with two notes (one grid, one quantum).
    fn synthetic_v2() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(SIGNATURE);
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&[0u8; 4]); // reserved
        out.extend_from_slice(&[0u8; 20]); // sha1
        out.extend_from_slice(&4000u32.to_le_bytes()); // last ms
        out.extend_from_slice(&2u32.to_le_bytes()); // note count
        out.extend_from_slice(&2u32.to_le_bytes()); // marker count
        out.push(3); // difficulty
        out.extend_from_slice(&7u16.to_le_bytes()); // star rating
        out.extend_from_slice(&[1, 0, 0]); // flags
        let ptr_pos = out.len();
        out.extend_from_slice(&[0u8; 80]); // 5 pointer pairs, patched below

        let s16 = |out: &mut Vec<u8>, s: &str| {
            out.extend_from_slice(&(s.len() as u16).to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        };
        s16(&mut out, "test_map_id");
        s16(&mut out, "Test Map");
        s16(&mut out, "Test Song");
        out.extend_from_slice(&1u16.to_le_bytes()); // mapper count
        s16(&mut out, "Mapper");

        // marker definitions: 1 definition "ssp_note" with one 0x07 value
        let defs_off = out.len() as u64;
        out.push(1);
        s16(&mut out, "ssp_note");
        out.push(1);
        out.push(0x07);
        out.push(0x00);
        let defs_len = out.len() as u64 - defs_off;

        // markers: grid note at 1000ms (1,2); quantum note at 2000ms
        let markers_off = out.len() as u64;
        out.extend_from_slice(&1000u32.to_le_bytes());
        out.push(0); // definition index
        out.push(0); // quantum flag off
        out.push(1);
        out.push(2);
        out.extend_from_slice(&2000u32.to_le_bytes());
        out.push(0);
        out.push(1); // quantum flag on
        out.extend_from_slice(&0.5f32.to_le_bytes());
        out.extend_from_slice(&1.5f32.to_le_bytes());
        let markers_len = out.len() as u64 - markers_off;

        // patch the definition/marker pointers (slots 3 and 4)
        out[ptr_pos + 48..ptr_pos + 56].copy_from_slice(&defs_off.to_le_bytes());
        out[ptr_pos + 56..ptr_pos + 64].copy_from_slice(&defs_len.to_le_bytes());
        out[ptr_pos + 64..ptr_pos + 72].copy_from_slice(&markers_off.to_le_bytes());
        out[ptr_pos + 72..ptr_pos + 80].copy_from_slice(&markers_len.to_le_bytes());
        out
    }

    #[test]
    fn parses_synthetic_v2() {
        let map = parse(&synthetic_v2()).unwrap();
        assert_eq!(map.meta.legacy_id, "test_map_id");
        assert_eq!(map.meta.song_name, "Test Song");
        assert_eq!(map.meta.mappers, vec!["Mapper".to_string()]);
        assert_eq!(map.meta.duration_ms, 4000);
        assert_eq!(map.notes.len(), 2);
        assert_eq!(map.notes[0].time_ms, 1000);
        assert_eq!((map.notes[0].x, map.notes[0].y), (1.0, 2.0));
        assert_eq!((map.notes[1].x, map.notes[1].y), (0.5, 1.5));
    }

    #[test]
    fn rejects_bad_signature() {
        assert!(parse(b"NOPE").is_err());
    }
}
