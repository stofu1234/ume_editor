# ume_editor

> **このリポジトリは https://github.com/stofu5678/ume_editor に移転しました。** ここは更新しません。

サクラエディタの操作感を、macOS・Windows・Linux で使えるように作り直すテキストエディタです。

> 開発を始めたばかりで、まだ動くものはありません。

サクラエディタとは別のプロジェクトです。サクラエディタのソースとヘルプ（zlib License）を仕様の参考にしています。

## 目指すもの

- サクラエディタのキー割り当てと操作感
- 巨大なファイルでも軽快に開く（2GB を超えるファイルは範囲ごとに読み込む）
- サクラエディタと同じ正規表現エンジン（Onigmo）
- Python や Ruby で書けるマクロとプラグイン

## ドキュメント

- [開発の計画](docs/roadmap.md)
- [決定事項（ADR）](docs/adr/README.md)

## ライセンス

次のどちらかを選んで利用できます。

- MIT License（[LICENSE-MIT](LICENSE-MIT)）
- Apache License 2.0（[LICENSE-APACHE](LICENSE-APACHE)）

サクラエディタのコードを移植した部分には、そのファイルに元の著作権表示と zlib License を残します。

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
