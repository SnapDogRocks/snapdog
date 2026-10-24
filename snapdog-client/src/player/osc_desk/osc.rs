//! The OSC subset the desks speak: messages with `i`, `f` and `s` arguments.

/// One OSC argument. Only the types the desks use.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Arg {
    Int(i32),
    Float(f32),
    Str(String),
}

/// Append an OSC string. The terminator counts towards the length, so a string whose
/// length plus terminator is already a multiple of four gets no padding: `/meters` is
/// exactly eight bytes. Padding to `4 - len % 4` instead adds four stray zero bytes there,
/// and the X32 rejects the packet with `OSC_ERROR: type tags not present`.
fn push_padded(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(s.as_bytes());
    buf.push(0);
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
}

pub(super) fn encode(address: &str, args: &[Arg]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(32);
    push_padded(&mut buf, address);
    // The tags are strings, not chars: Codacy's analyzer misreads `'i',` in a match
    // arm and then counts the rest of the file as part of this function.
    let mut tags = String::from(",");
    for arg in args {
        tags.push_str(match arg {
            Arg::Int(_) => "i",
            Arg::Float(_) => "f",
            Arg::Str(_) => "s",
        });
    }
    push_padded(&mut buf, &tags);
    for arg in args {
        match arg {
            Arg::Int(v) => buf.extend_from_slice(&v.to_be_bytes()),
            Arg::Float(v) => buf.extend_from_slice(&v.to_be_bytes()),
            Arg::Str(s) => push_padded(&mut buf, s),
        }
    }
    buf
}

fn read_padded(data: &[u8], pos: usize) -> Option<(String, usize)> {
    let rest = data.get(pos..)?;
    let end = rest.iter().position(|&b| b == 0)?;
    let s = std::str::from_utf8(&rest[..end]).ok()?.to_owned();
    Some((s, pos + (end + 1).div_ceil(4) * 4))
}

/// Parse a reply. Anything that is not plain `i`/`f`/`s` (meter blobs, other tags) is
/// rejected, which is also how stray packets are skipped while waiting for an answer.
pub(super) fn decode(data: &[u8]) -> Option<(String, Vec<Arg>)> {
    let (address, mut pos) = read_padded(data, 0)?;
    if !address.starts_with('/') {
        return None;
    }
    let (tags, next) = read_padded(data, pos)?;
    pos = next;
    let mut args = Vec::new();
    for tag in tags.strip_prefix(',')?.chars() {
        match tag {
            'i' => {
                let bytes = data.get(pos..pos + 4)?;
                args.push(Arg::Int(i32::from_be_bytes(bytes.try_into().ok()?)));
                pos += 4;
            }
            'f' => {
                let bytes = data.get(pos..pos + 4)?;
                args.push(Arg::Float(f32::from_be_bytes(bytes.try_into().ok()?)));
                pos += 4;
            }
            's' => {
                let (s, next) = read_padded(data, pos)?;
                args.push(Arg::Str(s));
                pos = next;
            }
            _ => return None,
        }
    }
    Some((address, args))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    // ── Against the X32 reference ──

    #[test]
    fn aligned_string_gets_no_padding() {
        let mut buf = Vec::new();
        push_padded(&mut buf, "/meters");
        assert_eq!(buf, b"/meters\0");
        let mut buf = Vec::new();
        push_padded(&mut buf, ",si");
        assert_eq!(buf, b",si\0");
    }

    #[test]
    fn meters_request_from_the_reference() {
        // X32 OSC reference p. 17: /meters~,si~/meters/6~~~16
        assert_eq!(
            encode("/meters", &[Arg::Str("/meters/6".into()), Arg::Int(16)]),
            hex("2f6d6574657273002c7369002f6d65746572732f3600000000000010")
        );
    }

    #[test]
    fn query_without_arguments() {
        assert_eq!(encode("/info", &[]), b"/info\0\0\0,\0\0\0");
    }

    #[test]
    fn fader_write_from_the_reference() {
        // p. 14: fader 0.8250
        let value = f32::from_be_bytes([0x3f, 0x53, 0x34, 0xcd]);
        assert_eq!(
            encode("/ch/01/mix/fader", &[Arg::Float(value)]),
            hex("2f63682f30312f6d69782f6661646572000000002c6600003f5334cd")
        );
    }

    #[test]
    fn info_reply_from_the_reference() {
        // p. 12: /info~~~,ssss~~~V2.05~~~osc-server~~X32RACK~2.12~~~~
        let data = b"/info\0\0\0,ssss\0\0\0V2.05\0\0\0osc-server\0\0X32RACK\x002.12\0\0\0\0";
        let (address, args) = decode(data).unwrap();
        assert_eq!(address, "/info");
        assert_eq!(
            args,
            ["V2.05", "osc-server", "X32RACK", "2.12"]
                .map(|s| Arg::Str(s.into()))
                .to_vec()
        );
    }

    #[test]
    fn round_trip_and_rejects() {
        let msg = encode(
            "/a",
            &[Arg::Int(5), Arg::Float(0.5), Arg::Str("hallo".into())],
        );
        assert_eq!(
            decode(&msg).unwrap().1,
            vec![Arg::Int(5), Arg::Float(0.5), Arg::Str("hallo".into())]
        );
        assert!(decode(b"").is_none());
        assert!(decode(b"noslash\0").is_none());
        assert!(decode(b"/a\0\0,f\0\0\x3f").is_none(), "truncated float");
        // Meter replies carry blobs; they must not be mistaken for an answer.
        assert!(decode(&hex("2f6d65746572732f360000002c62000000000004")).is_none());
    }
}
