pub(super) const WIDTH: f32 = 46.0;
pub(super) const HEIGHT: f32 = 109.0;
pub(super) const FILL: u32 = 0x1d1d1b;

pub(super) fn pixel(x: f32, y: f32, speaking: bool, step: usize) -> u32 {
    let accent = 0xff8a6b;
    let (mx, my) = ((x - 23.0).abs(), (y - 24.0).abs());
    let corner = (mx - 4.0).max(0.0).hypot((my - 4.0).max(0.0));
    if (4.5..=6.0).contains(&corner) {
        return 0xe8e6e1;
    }
    let wave = 24.0 - 2.4 * ((x - 15.0) * 0.77).sin();
    let on_wave = (15.0..31.0).contains(&x) && (y - wave).abs() < 1.0;
    if on_wave && (x as usize + step) % 12 < 8 {
        return accent;
    }
    for (i, height) in [10.0, 16.0, 8.0].into_iter().enumerate() {
        let center = 18.0 + i as f32 * 5.0;
        let bounce = 0.5 + 0.1 * ((step + i * 2) % 10).abs_diff(5) as f32;
        let quiet = if speaking { 1.0 } else { 0.75 };
        let half = height * bounce * quiet / 2.0;
        let radius = half.min(1.5);
        let end = ((y - 64.0).abs() - (half - radius)).max(0.0);
        if (x - center).hypot(end) < radius {
            return accent;
        }
    }
    let border = (x - 23.0).hypot(y - y.clamp(23.0, HEIGHT - 23.0)) > 22.0;
    [FILL, 0x3a3a37][usize::from(border)]
}
