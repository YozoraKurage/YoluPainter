//! 実行ファイル `yolupainter-cli`（Windows ではコンソールの実行ファイル）。

fn main() {
    let tokens: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(yolu_cli::cli::main_with(tokens));
}
