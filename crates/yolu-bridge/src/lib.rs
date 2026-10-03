//! Unity のエディタ拡張が読む DLL（C の関数。C# の P/Invoke は csbindgen で作る）。スタンドアロンにつなぐだけの薄い部品。
//!
//! Unity は一度読んだネイティブの DLL を手放さない（Windows ではファイルも掴んだまま）ので、この口は小さく変わりにくくする。
//! 最初に呼ぶのは `ylb_abi_version`（C# の知っている版と違えば、ほかの関数を呼ばない）。通信の形は yolu-protocol。

// C の関数の安全の決まり（ポインターは渡した長さのぶん読み書きできること・同じ領域を別のスレッドで同時に書かないこと）は
// ffi の頭にまとめて書いた。画素（4 バイト）を chunks_exact で回すのは読みやすさのため（yolu-core と同じ）。
#![allow(
    clippy::missing_safety_doc,
    clippy::chunks_exact_to_as_chunks,
    clippy::too_many_arguments
)]

mod copy;
pub mod ffi;
mod session;
mod testserver;

pub use ffi::*;
pub use testserver::YlbTestServerStats;
