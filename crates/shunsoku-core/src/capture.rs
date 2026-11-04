//! キャプチャ。どの発生源からの依頼も `request` の 1 か所で `CaptureRequest` にする（spec #32）。

pub mod hdr;
pub mod request;
pub mod rotation;
pub mod window;
