use lasercube::{
    animation::{Animation, Frame},
    LaserCube, LaserdockSample, XY_MAX, XY_MIN,
};

fn main() -> anyhow::Result<()> {
    let steps = 30;
    let delay_ms = 20;
    let mut frames = vec![];
    let y_start = 1.;
    let y_end = -1.;
    let y_delta = y_start - y_end;
    let mut even_odd = -1.;
    for step in 0..steps {
        let step_norm = step as f64 / (steps - 1) as f64;
        let step_eased = simple_easing::quad_out(step_norm as f32) as f64;
        let y = y_start - (y_delta as f64 * step_eased);
        let mut line = vec![];
        for line_point in 0..=20 {
            let x = (-1. + (line_point as f64) / 10.) * even_odd;
            line.push(LaserdockSample::new(
                ((x + 1.).powf(2.) * 50.) as u8,
                0,
                80,
                x,
                y,
            ));
        }
        frames.push(Frame::new(line));
        even_odd = even_odd * -1.;
    }

    let anim = Animation::new(frames, delay_ms);
    let mut device = LaserCube::open_first()?;
    device.set_dac_rate(5000)?;
    loop {
        anim.play(&device)?;
        // frames[0].draw(&device)?;
    }
}
