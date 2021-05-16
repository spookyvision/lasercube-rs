use std::{f64::consts::PI, mem::size_of};

use anyhow::Result;
use bytemuck::cast_slice;
use lasercube_rs::*;
use libusb::Context;
use log::debug;
fn main() -> Result<()> {
    pretty_env_logger::init();

    let context = Context::new()?;
    let it = context.devices()?;
    let (device, descriptor) = LaserCube::dd_from_context(it.iter())?;

    let mut lc = LaserCube::new(device, descriptor)?;
    lc.set_dac_rate(300)?;

    const NUM_POINTS: usize = 200;

    let mut circle: Vec<LaserdockSample> = Vec::with_capacity(NUM_POINTS * 2);

    for i in 0..2 * NUM_POINTS {
        let angle = (i % NUM_POINTS) as f64 / NUM_POINTS as f64 * PI * 2.;
        circle.push(LaserdockSample::new(
            (i % 40) as u8,
            (i % 90) as u8,
            (i % 130) as u8,
            angle.sin(),
            angle.cos(),
        ));
    }

    const SAMPLES_PER_BATCH: usize = 64 / (size_of::<LaserdockSample>() / size_of::<u8>());

    let circle = circle.as_slice();

    let mut idx = 0;

    loop {
        lc.send_samples(&circle[idx..idx + SAMPLES_PER_BATCH])?;
        idx = (idx + SAMPLES_PER_BATCH) % (circle.len() / 2);
    }

    Ok(())
}
