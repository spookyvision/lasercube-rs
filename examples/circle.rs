use std::{f64::consts::PI, mem::size_of};

use anyhow::Result;
use bytemuck::cast_slice;
use lasercube::*;
use log::debug;
fn main() -> Result<()> {
    pretty_env_logger::init();

    let mut lc = LaserCube::open_first()?;
    lc.set_dac_rate(30000)?;

    const NUM_POINTS: usize = 200;

    // instead of dealing with a ring buffer we'll just create two circles one after another
    // and send a slice of that

    const SAMPLE_SIZE: usize = size_of::<LaserdockSample>() / size_of::<u8>();
    const SAMPLES_PER_BATCH: usize = lasercube::BYTES_PER_BATCH / SAMPLE_SIZE;

    let mut circle: Vec<LaserdockSample> = Vec::with_capacity(NUM_POINTS + SAMPLES_PER_BATCH);
    debug!(
        "buffer len in samples: {}, bytes: {}",
        circle.capacity(),
        circle.capacity() * SAMPLE_SIZE
    );
    for i in 0..circle.capacity() {
        let angle = (i % NUM_POINTS) as f64 / NUM_POINTS as f64 * PI * 2.;
        circle.push(LaserdockSample::new(
            (i % 40) as u8,
            128 as u8,
            (i % 130) as u8,
            angle.sin(),
            angle.cos(),
        ));
    }

    let circle = circle.as_slice();

    let mut idx = 0;

    loop {
        lc.send_samples(&circle[idx..idx + SAMPLES_PER_BATCH])?;
        idx = (idx + SAMPLES_PER_BATCH) % (NUM_POINTS);
    }

    Ok(())
}
