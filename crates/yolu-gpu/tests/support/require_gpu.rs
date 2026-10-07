//! アダプターが無くて GPU の試験を飛ばすときの共通の口。環境変数 `YOLUPAINTER_REQUIRE_GPU` があれば、飛ばさず失敗にする
//! （CI の画面の試験のジョブが付ける。アダプターを取れない台で、GPU の試験が通った扱いになるのを防ぐ。`bake.rs` の `skipped` と同じ形）。

/// `what`（「キャンバスの GPU 試験」など）を、理由 `why` を標準エラーへ出して飛ばす。`YOLUPAINTER_REQUIRE_GPU` があれば落とす。
pub fn skipped(what: &str, why: &str) {
    eprintln!("{what}をスキップ: {why}");
    assert!(
        std::env::var_os("YOLUPAINTER_REQUIRE_GPU").is_none(),
        "YOLUPAINTER_REQUIRE_GPU があるのに GPU を使えない: {why}"
    );
}
