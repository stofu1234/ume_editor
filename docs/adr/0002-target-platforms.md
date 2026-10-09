# ADR-0002: 対象プラットフォーム

- 状態: 決定（初期開発で優先する対象は [ADR-0009](0009-initial-development-target.md)）
- 日付: 2026-10-08

## 決定

- macOS をメインのターゲットにする
- Windows と Linux のサポートも維持する
- CI で macOS・Windows・Linux のビルドとテストを常に通す
- OS 固有の処理は専用のモジュールに閉じ込める

## 開発環境

- UI の開発と確認（IME、文字の見え方）は Mac で行う
- コア（バッファ・文字コード・正規表現）は WSL2 の Linux（aarch64）でも開発できる
- Windows での確認と、サクラエディタを使った期待値の作成は Windows 実機で行う。実機は x64 版と ARM64 版の両方がある
