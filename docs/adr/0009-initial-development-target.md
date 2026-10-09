# ADR-0009: 初期開発の対象

- 状態: 決定
- 日付: 2026-10-09

## 背景

[ADR-0002](0002-target-platforms.md) では macOS をメインのターゲットにした。この方針は変えないが、初期開発のあいだは、ユーザーが普段使っている ARM64 版 Windows で作って確かめるほうが進めやすい。

## 決定

- 初期開発では、Windows ARM64（`aarch64-pc-windows-msvc`）で動かして確かめることを優先する
- 最終的なメインのターゲットは macOS のまま変えない。UI フレームワークも、macOS 寄りのクロスプラットフォームという方針（[ADR-0004](0004-ui-framework.md)）のまま進める
- UI の試作（[手順](../poc/ui-framework.md)）は、まず Windows ARM64 で行う。macOS に固有の確認項目（メニューバー、macOS の日本語入力など）は、Mac での作業を始めたときに行う
- CI では、Windows（x64・ARM64）、macOS（ARM64）、Linux（x64・ARM64）のビルドとテストを通す。x64 版の Windows は実機でも確認する

## 開発環境

- コードの編集とコアのテストは WSL2（aarch64 の Linux）で行う
- Windows 向けのビルドと起動は、Windows 側に入れた Rust（rustup）と Visual Studio の MSVC ARM64 ツールで行う
