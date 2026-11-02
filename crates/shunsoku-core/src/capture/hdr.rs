//! HDR のモニタの画像を SDR の画像にする変換（spec #34「HDR の変換」）。

/// scRGB の 1.0 の明るさ（nit）。
pub const SCRGB_WHITE_NITS: f32 = 80.0;

/// scRGB の値を、そのモニタの「SDR コンテンツの明るさ」（nit）が 1.0 になるように正規化する。
pub fn normalize(scrgb: f32, sdr_white_nits: f32) -> f32 {
    scrgb * SCRGB_WHITE_NITS / sdr_white_nits
}

/// 白より明るい部分が白に寄っていく速さ。最大の成分が白の `1 + HIGHLIGHT_ROLLOFF` 倍で、
/// 白との中間の色になる。
pub const HIGHLIGHT_ROLLOFF: f32 = 2.0;

/// 正規化した線形の RGB を SDR の範囲（0〜1）に収める。
///
/// - 最大の成分が 1 以下（SDR の範囲）なら、そのまま通す（負の値は 0 に切る）。
/// - 1 を超える色は、成分の比（色相）を保ったまま最大の成分を 1 にし、白より明るいほど
///   なだらかに白へ寄せる。寄せる割合 `w = (m - 1) / (m - 1 + HIGHLIGHT_ROLLOFF)` は m = 1 で 0
///   なので、白の前後で色が跳ばない。成分ごとに 1 で切ると、明るい色が黄や白に化けて
///   階調も消える（白飛び）。
///
/// シェーダー（`crates/shunsoku/src/hdr.rs` の HLSL）にも同じ式を書いている。変えるときは両方を変える。
pub fn tone_map(rgb: [f32; 3]) -> [f32; 3] {
    let rgb = rgb.map(|c| c.max(0.0));
    let m = rgb[0].max(rgb[1]).max(rgb[2]);
    if m <= 1.0 {
        return rgb;
    }
    let w = (m - 1.0) / (m - 1.0 + HIGHLIGHT_ROLLOFF);
    rgb.map(|c| {
        let base = c / m;
        base + (1.0 - base) * w
    })
}

/// scRGB の 1 画素（線形の R, G, B）を、SDR の 8bit sRGB の BGRA（アルファ 255）にする。
///
/// シェーダーは同じ値を 8bit の UNORM のテクスチャに書く（GPU が最も近い値に丸める）。
pub fn to_sdr_bgra(scrgb: [f32; 3], sdr_white_nits: f32) -> [u8; 4] {
    let [r, g, b] = tone_map(scrgb.map(|c| normalize(c, sdr_white_nits))).map(encode_srgb);
    [b, g, r, 255]
}

/// 0〜1 の線形の値を sRGB で符号化して 8bit にする。
fn encode_srgb(linear: f32) -> u8 {
    let v = if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    };
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdr_white_becomes_one() {
        // SDR コンテンツの明るさ 240 nit のモニタでは、scRGB の 3.0（240 nit）が白
        assert_eq!(normalize(3.0, 240.0), 1.0);
        assert_eq!(normalize(1.5, 240.0), 0.5);
        // 既定の 80 nit なら値は変わらない
        assert_eq!(normalize(0.25, 80.0), 0.25);
    }

    #[test]
    fn sdr_range_passes_unchanged() {
        for rgb in [
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [0.2, 0.5, 0.9],
            [1.0, 0.0, 0.3],
        ] {
            assert_eq!(tone_map(rgb), rgb);
        }
    }

    fn close(a: [f32; 3], b: [f32; 3], eps: f32) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= eps)
    }

    #[test]
    fn brighter_than_white_stays_within_range() {
        for rgb in [[1.5, 1.5, 1.5], [12.0, 3.0, 0.5], [100.0, 0.0, 0.0]] {
            let out = tone_map(rgb);
            assert!(
                out.iter().all(|c| (0.0..=1.0).contains(c)),
                "{rgb:?} -> {out:?}"
            );
        }
    }

    #[test]
    fn curve_is_continuous_at_white() {
        // 白をわずかに超えた値は、白の手前の値とほぼ同じ色になる（白を境に跳ばない）
        let below = [1.0, 0.6, 0.2];
        let above = [1.0001, 0.60006, 0.20002];
        assert!(close(tone_map(below), tone_map(above), 1e-3));
    }

    #[test]
    fn brighter_highlights_approach_white_gradually() {
        // 同じ色を明るくしていくと、各成分は減らずに増え、白に近づく（いきなり白にならない）
        let color = [1.0, 0.5, 0.1];
        let mut last = tone_map(color);
        for k in [1.5, 2.0, 4.0, 8.0, 64.0] {
            let out = tone_map(color.map(|c| c * k));
            assert!(
                out.iter().zip(last).all(|(o, l)| *o >= l),
                "x{k}: {out:?} < {last:?}"
            );
            last = out;
        }
        assert!(
            tone_map(color.map(|c| c * 1.5))[2] < 0.5,
            "not blown out just above white"
        );
        assert!(
            close(last, [1.0, 1.0, 1.0], 0.05),
            "very bright -> near white: {last:?}"
        );
    }

    #[test]
    fn negative_values_become_black() {
        // scRGB は sRGB の色域の外を負の値で表す。SDR では 0 に切る
        assert_eq!(tone_map([-0.2, 0.5, 0.5]), [0.0, 0.5, 0.5]);
    }

    #[test]
    fn pixel_becomes_8bit_srgb_bgra() {
        // 240 nit が白のモニタ: scRGB 3.0 は白、0 は黒
        assert_eq!(to_sdr_bgra([3.0, 3.0, 3.0], 240.0), [255, 255, 255, 255]);
        assert_eq!(to_sdr_bgra([0.0, 0.0, 0.0], 240.0), [0, 0, 0, 255]);
        // sRGB の 128 は線形で 0.2159。B, G, R の順に並ぶ
        assert_eq!(to_sdr_bgra([0.2159, 0.0, 1.0], 80.0), [255, 0, 128, 255]);
        // 白より明るい白は白（頭打ち）
        assert_eq!(to_sdr_bgra([30.0, 30.0, 30.0], 240.0), [255, 255, 255, 255]);
    }
}
