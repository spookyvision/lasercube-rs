use std::{f64::consts::PI, mem::size_of};

use anyhow::Result;
use bytemuck::cast_slice;
use lasercube::{LaserCube, LaserdockSample, SAMPLES_PER_BATCH};

use log::debug;
fn main() -> Result<()> {
    pretty_env_logger::init();

    let mut lc = LaserCube::open_first()?;
    lc.set_dac_rate(20000)?;

    const NUM_POINTS: usize = 200;
    let mut circle: Vec<LaserdockSample> = Vec::with_capacity(NUM_POINTS + SAMPLES_PER_BATCH);
    debug!(
        "buffer len in samples: {}, bytes: {}",
        circle.capacity(),
        circle.capacity() * SAMPLE_SIZE
    );
    let mut y = 1.0;
    let mut dir = true;
    for i in 0..circle.capacity() {
        let invert = if dir { 1.0 } else { -1.0 };
        let angle = (i % NUM_POINTS) as f64 / NUM_POINTS as f64 * PI * 2.;
        let x = ((i % 20) as f64 / 10. - 1.0) * invert;
        let r = ((x + 1.0) * 150.) as u8;

        if i % 20 == 0 {
            circle.push(LaserdockSample::new(0, 0, 0, x, y));
            circle.push(LaserdockSample::new(0, 0, 0, x, y));
            circle.push(LaserdockSample::new(0, 0, 0, x, y));
            circle.push(LaserdockSample::new(0, 0, 0, x, y));
            circle.push(LaserdockSample::new(0, 0, 0, x, y));
            circle.push(LaserdockSample::new(0, 0, 0, x, y));
            circle.push(LaserdockSample::new(0, 0, 0, x, y));
        }

        circle.push(LaserdockSample::new(r, 0, 10, x, y));

        debug!("{:.2} {:.2} {}", x, y, r);

        if i % 20 == 19 {
            circle.push(LaserdockSample::new(0, 0, 0, x, y));
            circle.push(LaserdockSample::new(0, 0, 0, x, y));
            circle.push(LaserdockSample::new(0, 0, 0, x, y));
            circle.push(LaserdockSample::new(0, 0, 0, x, y));
            circle.push(LaserdockSample::new(0, 0, 0, x, y));
            circle.push(LaserdockSample::new(0, 0, 0, x, y));
            circle.push(LaserdockSample::new(0, 0, 0, x, y));
            debug!("--------");
            y -= 0.2;
            dir = !dir;
        }
    }

    let circle = circle.as_slice();

    let mut idx = 0;

    loop {
        lc.send_samples(&circle[idx..idx + SAMPLES_PER_BATCH])?;
        idx = (idx + SAMPLES_PER_BATCH) % (NUM_POINTS);
    }

    Ok(())
}
