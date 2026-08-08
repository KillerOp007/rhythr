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

    /// The same, with a 32-bit length (type 0x0b). Custom fields use either.
    fn string32(&mut self) -> Result<String> {
        let n = self.u32()? as usize;
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
        self.skip_value_at(type_id, 0)
    }

    /// How deep marker arrays may nest. Real files use one level at most;
    /// the limit exists because the element type of an array can be another
    /// array, so a crafted file could otherwise drive this recursion until
    /// the stack overflows: an abort no caller can catch.
    const MAX_ARRAY_DEPTH: u32 = 8;

    fn skip_value_at(&mut self, type_id: u8, depth: u32) -> Result<()> {
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
                if depth >= Self::MAX_ARRAY_DEPTH {
                    return Err(Error::Malformed(
                        "sspm: marker arrays nested too deeply".into(),
                    ));
                }
                let sub = self.u8()?;
                let count = self.u16()?;
                // Type 0x00 is zero bytes wide, so an array of it consumes
                // nothing at all: looping 65535 times would be pure waste,
                // and nesting such arrays multiplies out to a hang. Every
                // other element type advances the cursor, which bounds the
                // total work by the file size.
                if sub != 0x00 {
                    for _ in 0..count {
                        self.skip_value_at(sub, depth + 1)?;
                    }
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
    Ok(normalized(Map {
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
    }))
}

fn normalized(mut map: Map) -> Map {
    map.meta.normalize();
    map
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

    // Best effort, deliberately: this block carries an optional display name
    // and nothing the map needs. It also sits in its own addressed region, so
    // failing to walk it says nothing about the notes, the audio or the
    // cover, which are each reached by their own pointer.
    //
    // Measured against the published ranked catalogue (899 maps): six of them
    // have a custom block this walk cannot follow (it reads a field length of
    // 256 out of a 13-byte block), and every one of those six is otherwise
    // perfect, full note count, playable audio, valid cover. Refusing the
    // whole map over a difficulty label was the wrong trade.
    //
    // The name itself comes in two shapes in the wild: a u16-length string
    // (0x09, 528 of those maps) and a u32-length one (0x0b, 16 of them). Only
    // the first was read before, so those 16 lost their name for no reason.
    let custom_difficulty_name = (|| -> Result<String> {
        if custom_len == 0 {
            return Ok(String::new());
        }
        let mut c = Reader::new(slice_at(data, custom_off, custom_len)?);
        let fields = c.u16()?;
        let mut found = String::new();
        for _ in 0..fields {
            let name = c.string16()?;
            let type_id = c.u8()?;
            match (name.as_str(), type_id) {
                ("difficulty_name", 0x09) => found = c.string16()?,
                ("difficulty_name", 0x0b) => found = c.string32()?,
                _ => c.skip_value(type_id)?,
            }
        }
        Ok(found)
    })()
    .unwrap_or_default();

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

    Ok(normalized(Map {
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
    }))
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

    /// A minimal but complete v2 file, built here rather than taken from a
    /// real map: header, one note, and whatever custom block the caller wants
    /// to put in front of it.
    fn v2_file(custom: &[u8]) -> Vec<u8> {
        let mut head = Vec::new();
        head.extend_from_slice(b"SS+m");
        head.extend_from_slice(&2u16.to_le_bytes());
        head.extend_from_slice(&[0; 4]); // reserved
        head.extend_from_slice(&[0; 20]); // sha1, unverified
        head.extend_from_slice(&1000u32.to_le_bytes()); // last ms
        head.extend_from_slice(&1u32.to_le_bytes()); // notes
        head.extend_from_slice(&1u32.to_le_bytes()); // markers
        head.push(3); // difficulty
        head.extend_from_slice(&0u16.to_le_bytes()); // stars
        head.extend_from_slice(&[0, 0, 0]); // audio/cover/mod flags

        let strings = {
            let mut v = Vec::new();
            for s in ["mapid", "map name", "song name"] {
                v.extend_from_slice(&(s.len() as u16).to_le_bytes());
                v.extend_from_slice(s.as_bytes());
            }
            v.extend_from_slice(&1u16.to_le_bytes()); // one mapper
            v.extend_from_slice(&6u16.to_le_bytes());
            v.extend_from_slice(b"mapper");
            v
        };
        let defs = {
            let mut v = vec![1u8]; // one definition
            v.extend_from_slice(&8u16.to_le_bytes());
            v.extend_from_slice(b"ssp_note");
            v.push(1); // one value
            v.push(0x07); // of type position
            v.push(0); // terminator
            v
        };
        let markers = {
            let mut v = Vec::new();
            v.extend_from_slice(&500u32.to_le_bytes());
            v.push(0); // definition index
            v.push(0); // grid position
            v.extend_from_slice(&[1, 1]); // x, y
            v
        };

        // Pointers are absolute, so the layout has to be decided first.
        let ptr_bytes = 5 * 16;
        let base = head.len() + ptr_bytes + strings.len();
        let custom_off = base;
        let defs_off = custom_off + custom.len();
        let markers_off = defs_off + defs.len();

        let mut out = head;
        for (off, len) in [
            (custom_off, custom.len()),
            (0, 0), // audio
            (0, 0), // cover
            (defs_off, defs.len()),
            (markers_off, markers.len()),
        ] {
            out.extend_from_slice(&(off as u64).to_le_bytes());
            out.extend_from_slice(&(len as u64).to_le_bytes());
        }
        out.extend_from_slice(&strings);
        out.extend_from_slice(custom);
        out.extend_from_slice(&defs);
        out.extend_from_slice(&markers);
        out
    }

    /// Six maps in the published ranked catalogue carry a custom block this
    /// parser cannot walk (a field length of 256 inside a 13-byte block), and
    /// all six are otherwise perfect. The block holds an optional display
    /// name and nothing else, so it must never take the map down with it.
    #[test]
    fn an_unwalkable_custom_block_costs_the_name_and_nothing_else() {
        // Claims four fields, then runs out of block half way through one.
        let broken = vec![
            0x04, 0x00, 0x01, 0x00, 0x02, 0x00, 0x00, 0x01, 0x00, 0x5f, 0x01, 0x00, 0x30,
        ];
        let map = parse(&v2_file(&broken)).expect("a broken custom block must not fail the map");
        assert_eq!(map.notes.len(), 1, "the notes are in their own block");
        assert_eq!(map.meta.custom_difficulty_name, "");
        assert_eq!(map.meta.title, "map name");
    }

    /// The difficulty name comes in two shapes in the wild: 528 maps of that
    /// catalogue use a u16 length, 16 use a u32 one. Only the first was read.
    #[test]
    fn a_difficulty_name_is_read_whichever_length_it_carries() {
        for (type_id, len_bytes) in [(0x09u8, vec![4u8, 0]), (0x0bu8, vec![4u8, 0, 0, 0])] {
            let mut custom = Vec::new();
            custom.extend_from_slice(&1u16.to_le_bytes());
            custom.extend_from_slice(&15u16.to_le_bytes());
            custom.extend_from_slice(b"difficulty_name");
            custom.push(type_id);
            custom.extend_from_slice(&len_bytes);
            custom.extend_from_slice(b"Hard");
            let map = parse(&v2_file(&custom)).expect("parses");
            assert_eq!(
                map.meta.custom_difficulty_name, "Hard",
                "type 0x{type_id:02x}"
            );
        }
    }

    /// A hostile marker value: `depth` nested arrays, the innermost one an
    /// array of the zero-width type. Parsed naively this either overflows
    /// the stack or spins 65535^depth times.
    fn nested_array_value(depth: usize) -> Vec<u8> {
        let mut v = Vec::new();
        for _ in 0..depth {
            v.push(0x0c); // element type: another array
            v.extend_from_slice(&1u16.to_le_bytes());
        }
        v.push(0x00); // innermost element type: zero bytes wide
        v.extend_from_slice(&u16::MAX.to_le_bytes());
        v
    }

    #[test]
    fn deeply_nested_marker_arrays_are_rejected_not_fatal() {
        let bytes = nested_array_value(64);
        let mut r = Reader::new(&bytes);
        let err = r.skip_value(0x0c).unwrap_err();
        assert!(
            format!("{err}").contains("nested too deeply"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn zero_width_element_arrays_do_not_spin() {
        // One array of 65535 zero-width elements. The type byte is the
        // argument, so the value itself is just element type + count: the
        // reader must consume those three bytes and stop, not loop over
        // 65535 elements of nothing.
        let mut v = vec![0x00];
        v.extend_from_slice(&u16::MAX.to_le_bytes());
        let mut r = Reader::new(&v);
        r.skip_value(0x0c).unwrap();
        assert_eq!(r.pos, v.len());
    }

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

    #[test]
    fn placeholder_song_name_falls_back_to_title() {
        let mut meta = crate::map::MapMeta {
            song_name: "Artist Name - Song Name".into(),
            title: "DECO*27 - Rabbit Hole".into(),
            ..Default::default()
        };
        meta.normalize();
        assert_eq!(meta.song_name, "DECO*27 - Rabbit Hole");

        let mut kept = crate::map::MapMeta {
            song_name: "Real Artist - Real Song".into(),
            title: "anything".into(),
            ..Default::default()
        };
        kept.normalize();
        assert_eq!(kept.song_name, "Real Artist - Real Song");

        // Both placeholders: leave it alone (nothing better to show).
        let mut both = crate::map::MapMeta {
            song_name: "artist name - song name".into(),
            title: "".into(),
            ..Default::default()
        };
        both.normalize();
        assert_eq!(both.song_name, "artist name - song name");
    }
}
