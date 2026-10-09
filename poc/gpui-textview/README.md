# GPUI テキスト表示の試作

[ADR-0004](../../docs/adr/0004-ui-framework.md) を確定するための、捨てる前提の試作。結果は [docs/poc/ui-framework-result.md](../../docs/poc/ui-framework-result.md)。

本体のワークスペースには含めていない。このディレクトリで `cargo run --release` する。Windows では Visual Studio の C++ ツール（ARM64 なら ARM64 用）が必要。

- F7: フォントの切り替え、Ctrl+= / Ctrl+-: 文字の大きさ、F8: 100 万行のテキストを作る、Ctrl+O: 開く
- `cargo run --release --bin minimal`: メモリを比べるための最小のウィンドウ
- 環境変数 `POC_NO_MENU` / `POC_NO_ROOT` / `POC_NO_TEXT` / `POC_UI_FONT=<フォント名>`: メモリを測るための切り替え
- 環境変数 `POC_MEMSTAT=<秒>`: 起動してその秒数が経ったら、メモリの内訳を `memstat-<プロセスID>.txt` に書き出す（Windows のみ。Rust のヒープ、Win32 のヒープ、領域の種類ごとの量、大きい確保の一覧）
- 環境変数 `RUST_LOG=info`: GPUI のログ（選ばれた GPU など）を標準エラーに出す
- 環境変数 `POC_TEXT_MODE=grayscale`: 文字をグレースケールで描く（既定は Windows の設定に従い、ClearType ならサブピクセル）

## GPU ドライバーでメモリが増える問題の回避

Qualcomm Adreno（Windows ARM64）では、GPUI が文字をテクスチャに書き込むたびにメモリが約 4 MB 増える（[結果](../../docs/poc/ui-framework-result.md) を参照）。`patches/gpui-pre-windows-atlas-upload.diff` で回避できる。試すには次のようにする。

1. `~/.cargo/registry/src/*/gpui-pre-windows-0.3.8` を任意の場所に写し、そこで `patch -p1 < .../patches/gpui-pre-windows-atlas-upload.diff`
2. `Cargo.toml` の末尾に `[patch.crates-io]` と `gpui-pre-windows = { path = "<写した場所>" }` を足してビルドする（コミットはしない）
