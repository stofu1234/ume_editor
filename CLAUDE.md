# CLAUDE.md

このリポジトリで作業する AI エージェント向けの指示です。

## プロジェクト

サクラエディタの操作感を macOS・Windows・Linux で再現するテキストエディタ。Rust で書き、MIT OR Apache-2.0 で公開する。

- 計画と現在のフェーズ: [docs/roadmap.md](docs/roadmap.md)
- 決定事項: [docs/adr/](docs/adr/README.md)。ADR と食い違う変更をするときは、先に ADR を更新するか、ユーザーに確認する
- 初期開発では Windows ARM64 で動かして確かめることを優先する（[ADR-0009](docs/adr/0009-initial-development-target.md)）

## ビルドとテスト

Rust のバージョンは `rust-toolchain.toml` で固定している。push の前に、次の 3 つを通す（CI と同じ内容）。

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

- CI（`.github/workflows/ci.yml`）は Windows x64・ARM64、macOS ARM64、Linux x64・ARM64 で回す
- クレートは `crates/` に置く。`ume-core` は UI に依存しないコア

## 設計のルール

- コア（バッファ・文字コード・検索・編集操作）は UI に依存させない。依存の向きは UI → コアの一方向にする
- 操作はすべて名前付きのコマンドとして実装する。1 コマンド 1 ファイルにし、全員が書き換える一覧（巨大な `match` など）を作らない。キー割り当て・メニュー・マクロはコマンドを呼ぶ
- 文書へのアクセスは範囲指定で行う。全文を 1 つの文字列で受け渡す API を作らない（2GB を超えるファイルで使えないため）
- OS 固有の処理は専用のモジュールに閉じ込める。macOS がメインだが、Windows と Linux でもビルドとテストが通る状態を保つ

## ライセンスのルール

- 依存を追加するときは、ライセンスが MIT / Apache-2.0 と両立するか確認する。GPL 系（GPL・LGPL・AGPL）のものは使わない
- Zed のエディタ部分（GPL / AGPL のクレート）からコードを写さない。参考にしてよいのは GPUI 本体と gpui-component（どちらも Apache-2.0）まで
- サクラエディタ（zlib License）のコードを移植するときは、ファイルの先頭に移植元のファイル名、元の著作権表示、zlib License の全文を残し、改変したことを明記する

## サクラエディタの仕様の調べ方

上流は https://github.com/sakura-editor/sakura 。作業用の一時ディレクトリに `git clone --depth 1` して読む。

- コマンド一覧: `sakura_core/Funccode_x.hsrc`
- 既定のキー割り当て: `sakura_core/func/CKeyBind.cpp`
- 正規表現の構文: `help/sakura/res/HLP000089.html`
- 検索の実装: `sakura_core/agent/CSearchAgent.cpp`

サクラの挙動を再現したコードには、根拠にしたソースかヘルプの場所をコメントで残す。

## 書き方

- ドキュメント（README と docs/）は日本語で書く
- コードの識別子とコメントは英語で書く
