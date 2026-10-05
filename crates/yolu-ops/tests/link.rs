//! 起動中のアプリへの通信の枠の試験: 要求・返事の JSON と枠の往復、版の違いの断り、大きすぎる中身、実際の流れ（ソケット）での往復。
//! 経路そのもの（待ち受け・鍵の確かめ合い）は含まない。

mod common;

use std::io::Read;
use std::net::{TcpListener, TcpStream};

use common::*;
use serde_json::{json, Value};
use yolu_ops::link::*;
use yolu_ops::reply::{PreviewInfo, Reply};
use yolu_ops::value::Bytes;
use yolu_ops::{execute, parse_command, ErrorCode, OpError, COMMAND_VERSION};
use yolu_protocol::frame::{Frame, FrameReader};

fn request(id: u64, command: Value) -> Request {
    Request { id, command: parse_command(&command).unwrap() }
}

/// 枠が 1 つそろうところまで読む（そろわなければ試験の失敗）。
fn frame_of(reader: &mut FrameReader, stream: &mut impl Read) -> Frame {
    match read_frame(reader, stream).unwrap() {
        Received::Frame(frame) => frame,
        other => panic!("枠がそろわなかった: {other:?}"),
    }
}

#[test]
fn the_link_name_is_valid_and_separate_from_live_link() {
    assert!(yolu_protocol::link::valid_link_name(LINK_NAME));
    assert_ne!(LINK_NAME, yolu_protocol::DEFAULT_LINK_NAME, "Live Link とは別の名前（別のソケット・別の鍵のファイル）");
    assert!(LINK_NAME.len() <= 64);
    assert_ne!(KIND_REQUEST, KIND_RESPONSE);
}

#[test]
fn requests_and_responses_round_trip_through_json() {
    for (i, command) in [
        json!({"command": "doc.info"}),
        json!({"command": "layer.set", "args": {"layer": "Base", "opacity": 0.5}}),
        json!({"command": "effect.add", "args": {"layer": "x", "kind": "blur", "values": {"radius": 4}}}),
    ]
    .into_iter()
    .enumerate()
    {
        let r = request(i as u64 + 7, command);
        let json = r.to_json();
        assert_eq!(json["v"], json!(COMMAND_VERSION));
        assert_eq!(json["id"], json!(i as u64 + 7));
        assert_eq!(Request::from_json(&json).unwrap(), r);
        // 枠を通しても同じ
        let bytes = encode_request(&r).unwrap();
        let mut reader = FrameReader::new();
        let frame = frame_of(&mut reader, &mut bytes.as_slice());
        assert_eq!(frame.kind, KIND_REQUEST);
        assert_eq!(decode_request(&frame).unwrap(), r);
    }
    let fx = Fixture::new("link-json");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let reply = ok(&mut host, json!({"command": "set.info"}));
    let good = Response::ok(3, reply);
    assert_eq!(Response::from_json(&good.to_json()).unwrap(), good);
    let bad = Response::err(4, OpError::new(ErrorCode::NotFound, "無い", "missing").with_data(json!({"x": 1})));
    let json = bad.to_json();
    assert_eq!((json["ok"].clone(), json["v"].clone()), (json!(false), json!(COMMAND_VERSION)));
    assert_eq!(Response::from_json(&json).unwrap(), bad);
    let bytes = encode_response(&good);
    let mut reader = FrameReader::new();
    let frame = frame_of(&mut reader, &mut bytes.as_slice());
    assert_eq!(frame.kind, KIND_RESPONSE);
    assert_eq!(decode_response(&frame).unwrap(), good);
}

#[test]
fn a_different_command_version_or_a_missing_field_is_refused_with_a_reason() {
    let good = request(1, json!({"command": "doc.info"})).to_json();
    for v in [json!(0), json!(2), json!("1"), json!(null)] {
        let mut json = good.clone();
        json["v"] = v.clone();
        let e = Request::from_json(&json).unwrap_err();
        assert_eq!(e.code, ErrorCode::UnsupportedVersion, "v = {v}");
        assert_eq!(e.data.unwrap()["supported"], json!([COMMAND_VERSION]));
    }
    // 通信の要求は、版を省けない（命令の JSON は省ける）
    let mut json = good.clone();
    json.as_object_mut().unwrap().remove("v");
    assert_eq!(Request::from_json(&json).unwrap_err().code, ErrorCode::UnsupportedVersion);
    parse_command(&json).unwrap();
    // id が無い・数でない
    for broken in [json!({"v": 1, "command": "doc.info"}), json!({"v": 1, "id": "x", "command": "doc.info"})] {
        assert_eq!(Request::from_json(&broken).unwrap_err().code, ErrorCode::InvalidRequest);
    }
    // 知らない命令・引数の誤りは、要求として読む所で断る
    let e = Request::from_json(&json!({"v": 1, "id": 1, "command": "nope"})).unwrap_err();
    assert_eq!(e.code, ErrorCode::UnknownCommand);
    let e = Request::from_json(&json!({"v": 1, "id": 1, "command": "layer.get", "args": {"extra": 1}})).unwrap_err();
    assert_eq!(e.code, ErrorCode::InvalidRequest);
    // 返事の版・欄
    let reply = Response::ok(1, Reply::History(yolu_ops::reply::HistoryInfo { undo_count: 0, redo_count: 0, can_undo: false, can_redo: false }));
    for v in [json!(2), json!(null)] {
        let mut json = reply.to_json();
        json["v"] = v;
        assert_eq!(Response::from_json(&json).unwrap_err().code, ErrorCode::UnsupportedVersion);
    }
    for field in ["v", "id", "ok", "reply"] {
        let mut json = reply.to_json();
        json.as_object_mut().unwrap().remove(field);
        assert!(Response::from_json(&json).is_err(), "{field}");
    }
}

#[test]
fn frames_of_the_wrong_kind_or_with_broken_json_are_refused() {
    let r = request(1, json!({"command": "doc.info"}));
    let mut reader = FrameReader::new();
    let frame = frame_of(&mut reader, &mut encode_request(&r).unwrap().as_slice());
    assert_eq!(decode_response(&frame).unwrap_err().code, ErrorCode::InvalidRequest);
    let broken = Frame { kind: KIND_REQUEST, flags: 0, payload: b"{not json".to_vec() };
    assert_eq!(decode_request(&broken).unwrap_err().code, ErrorCode::InvalidRequest);
    // 頭の合言葉が違う流れは、区切りが分からないので断る
    let mut reader = FrameReader::new();
    let e = read_frame(&mut reader, &mut &b"NOPE0000000000000000"[..]).unwrap_err();
    assert_eq!(e.code, ErrorCode::Io);
}

/// 1 バイトずつしか返さない読み手でも、枠を組み立てられる（流れの途中で切れた読みを失わない）。
struct Trickle<'a>(&'a [u8]);
impl Read for Trickle<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let Some((first, rest)) = self.0.split_first() else { return Ok(0) };
        buf[0] = *first;
        self.0 = rest;
        Ok(1)
    }
}

#[test]
fn a_frame_is_reassembled_from_a_stream_that_delivers_one_byte_at_a_time() {
    let r = request(9, json!({"command": "layer.set", "args": {"layer": "Base", "name": "名前"}}));
    let mut both = encode_request(&r).unwrap();
    both.extend(encode_request(&request(10, json!({"command": "doc.info"}))).unwrap());
    let mut stream = Trickle(&both);
    let mut reader = FrameReader::new();
    let a = frame_of(&mut reader, &mut stream);
    let b = frame_of(&mut reader, &mut stream);
    assert_eq!(decode_request(&a).unwrap(), r);
    assert_eq!(decode_request(&b).unwrap().id, 10);
    assert_eq!(read_frame(&mut reader, &mut stream).unwrap(), Received::Closed);
}

/// 台本どおりに返す読み手: 決めたバイト列・時間切れを順に返し、尽きたら相手が閉じた（0 バイト）。
struct Script(std::collections::VecDeque<Step>);
enum Step {
    Bytes(Vec<u8>),
    Error(std::io::ErrorKind),
}
impl Read for Script {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.0.pop_front() {
            None => Ok(0),
            Some(Step::Error(kind)) => Err(kind.into()),
            Some(Step::Bytes(mut bytes)) => {
                let n = bytes.len().min(buf.len());
                buf[..n].copy_from_slice(&bytes[..n]);
                if n < bytes.len() {
                    self.0.push_front(Step::Bytes(bytes.split_off(n)));
                }
                Ok(n)
            }
        }
    }
}
fn script(steps: Vec<Step>) -> Script {
    Script(steps.into())
}

#[test]
fn a_read_timeout_is_idle_not_the_end_of_the_link_and_keeps_the_partial_frame() {
    let bytes = encode_request(&request(21, json!({"command": "layer.get", "args": {"layer": "Base"}}))).unwrap();
    let (head, tail) = bytes.split_at(bytes.len() / 2);
    for kind in [std::io::ErrorKind::TimedOut, std::io::ErrorKind::WouldBlock, std::io::ErrorKind::Interrupted] {
        let mut stream = script(vec![
            Step::Error(kind),
            Step::Bytes(head.to_vec()),
            Step::Error(kind),
            Step::Bytes(tail.to_vec()),
        ]);
        let mut reader = FrameReader::new();
        // 何も届いていない間の時間切れ
        assert_eq!(read_frame(&mut reader, &mut stream).unwrap(), Received::Idle, "{kind:?}");
        // 枠の途中までで時間切れ: 読んだ分は失わず、つながりが終わったとも見ない
        assert_eq!(read_frame(&mut reader, &mut stream).unwrap(), Received::Idle, "{kind:?}");
        assert_eq!(reader.pending(), head.len());
        // 残りが届けば、同じ読み手で枠がそろう
        let frame = frame_of(&mut reader, &mut stream);
        assert_eq!(decode_request(&frame).unwrap().id, 21);
        // そのあと相手が閉じたら、枠の切れ目の閉じ
        assert_eq!(read_frame(&mut reader, &mut stream).unwrap(), Received::Closed);
    }
}

#[test]
fn closing_between_frames_is_closed_and_closing_inside_a_frame_is_an_error() {
    // 何も読まずに閉じた・枠をそろえたあとで閉じた: きれいな閉じ
    let mut reader = FrameReader::new();
    assert_eq!(read_frame(&mut reader, &mut &b""[..]).unwrap(), Received::Closed);
    let bytes = encode_request(&request(1, json!({"command": "doc.info"}))).unwrap();
    let mut stream = bytes.as_slice();
    assert_eq!(decode_request(&frame_of(&mut reader, &mut stream)).unwrap().id, 1);
    assert_eq!(read_frame(&mut reader, &mut stream).unwrap(), Received::Closed);
    // 頭の途中・中身の途中で閉じた: 要求が欠けたので、閉じとして流さず、何バイト目かを言う誤り
    for cut in [3, 12, bytes.len() - 1] {
        let mut reader = FrameReader::new();
        let e = read_frame(&mut reader, &mut &bytes[..cut]).unwrap_err();
        assert_eq!(e.code, ErrorCode::Io, "{cut}");
        assert!(e.message.en.contains("middle of a frame") && e.message.ja.contains("途中"), "{cut}: {}", e.message.en);
        assert!(e.message.en.contains(&format!("{cut} bytes")), "{}", e.message.en);
    }
}

#[test]
fn a_message_over_the_limit_is_refused_before_it_is_sent() {
    // 返事: 上限を超える見本の画像は、送れない理由の誤りの返事に替わる（同じ id で、読める）
    let huge = Reply::Preview(PreviewInfo {
        set: "s".into(),
        channel: "Color".into(),
        width: 1,
        height: 1,
        source_width: 1,
        source_height: 1,
        png: Bytes(vec![0; MAX_PAYLOAD]),
        inactive_effects: vec![],
    });
    let bytes = encode_response(&Response::ok(5, huge));
    let mut reader = FrameReader::new();
    let frame = frame_of(&mut reader, &mut bytes.as_slice());
    let response = decode_response(&frame).unwrap();
    assert_eq!(response.id, 5);
    let e = response.outcome.unwrap_err();
    assert_eq!(e.code, ErrorCode::Budget);
    assert!(e.message.ja.contains("大きすぎます") && e.message.en.contains("too large"));
    // 要求: 送る前に断る
    let command = parse_command(&json!({"command": "layer.get", "args": {"layer": "x".repeat(MAX_PAYLOAD)}})).unwrap();
    assert_eq!(encode_request(&Request { id: 1, command }).unwrap_err().code, ErrorCode::Budget);
}

/// 実際の流れ（ループバックの TCP。試験の道具で、経路の実装ではない）で、要求を送って返事を受ける。
#[test]
fn requests_travel_over_a_stream_to_a_host_and_the_replies_come_back_in_order() {
    let fx = Fixture::new("link-stream");
    fx.project("a.ylp");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server_fx_dir = fx.dir.clone();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut host = yolu_ops::FileHost::new(yolu_ops::PathPolicy::new(&server_fx_dir).unwrap());
        let mut reader = FrameReader::new();
        let mut served = 0;
        loop {
            let frame = match read_frame(&mut reader, &mut stream).unwrap() {
                Received::Frame(frame) => frame,
                Received::Idle => continue,
                Received::Closed => break,
            };
            let response = match decode_request(&frame) {
                Ok(request) => Response { id: request.id, outcome: execute(&mut host, &request.command) },
                Err(error) => Response::err(0, error),
            };
            write_bytes(&mut stream, &encode_response(&response)).unwrap();
            served += 1;
        }
        served
    });
    let mut stream = TcpStream::connect(addr).unwrap();
    let mut reader = FrameReader::new();
    let mut ask = |id: u64, command: Value| -> Response {
        write_bytes(&mut stream, &encode_request(&request(id, command)).unwrap()).unwrap();
        let frame = frame_of(&mut reader, &mut stream);
        decode_response(&frame).unwrap()
    };
    // 文書が無い間は、理由つきの誤り
    let r = ask(1, json!({"command": "doc.info"}));
    assert_eq!((r.id, r.outcome.unwrap_err().code), (1, ErrorCode::NoDocument));
    let r = ask(2, json!({"command": "doc.open", "args": {"path": "a.ylp"}}));
    assert!(matches!(r.outcome, Ok(Reply::Doc(_))));
    let r = ask(3, json!({"command": "layer.add", "args": {"kind": "paint", "name": "Over the wire"}}));
    let Ok(Reply::Edited(edited)) = r.outcome else { panic!("{r:?}") };
    assert_eq!(r.id, 3);
    assert!(edited.layer.is_some() && edited.undo_count == 1);
    // 見本の画像（PNG）が枠に載って返る
    let r = ask(4, json!({"command": "preview", "args": {"max_edge": 32}}));
    let Ok(Reply::Preview(p)) = r.outcome else { panic!() };
    assert_eq!((p.width, p.height), (32, 24));
    assert_eq!(&p.png.0[..8], b"\x89PNG\r\n\x1a\n");
    // 断りも同じ道で返る
    let r = ask(5, json!({"command": "layer.delete", "args": {"layer": "Base"}}));
    assert_eq!((r.id, r.outcome.unwrap_err().code), (5, ErrorCode::ConfirmRequired));
    drop(ask);
    drop(stream);
    assert_eq!(server.join().unwrap(), 5);
}
