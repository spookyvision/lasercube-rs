use std::{f64::consts::PI, mem::size_of};

use anyhow::Result;
use bytemuck::cast_slice;
use lasercube::*;
use lasy::{
    euler_graph_to_euler_circuit, interpolate_euler_circuit, point_graph_to_euler_graph,
    points_to_segments, segments_to_point_graph, InterpolationConfig,
};
use log::debug;
fn main() -> Result<()> {
    pretty_env_logger::init();

    let mut lc = LaserCube::open_first()?;
    lc.set_dac_rate(30000)?;

    const NUM_POINTS: usize = 150;

    // instead of dealing with a ring buffer we'll just create two circles one after another
    // and send a slice of that

    const SAMPLE_SIZE: usize = size_of::<LaserdockSample>() / size_of::<u8>();
    const SAMPLES_PER_BATCH: usize = lasercube::BYTES_PER_BATCH / SAMPLE_SIZE;

    let mut circle: Vec<LaserdockSample> = vec![];

    for i in 0..NUM_POINTS {
        let angle = (i % NUM_POINTS) as f64 / NUM_POINTS as f64 * PI * 2. - PI / 2.;
        if i == 0 {
            circle.push(LaserdockSample::new(0, 0, 0, angle.sin(), angle.cos()));
        }
        circle.push(LaserdockSample::new(
            (i % 40) as u8,
            128 as u8,
            (i % 130) as u8,
            angle.sin(),
            angle.cos(),
        ));
    }

    for i in 0..NUM_POINTS {
        let angle = (i % NUM_POINTS) as f64 / NUM_POINTS as f64 * PI * 2. - PI / 2.;
        if i == 0 {
            circle.push(LaserdockSample::new(
                0,
                0,
                0,
                angle.sin() / 2.,
                angle.cos() / 2.,
            ));
        }
        circle.push(LaserdockSample::new(
            (i % 40) as u8,
            128 as u8,
            (i % 130) as u8,
            angle.sin() / 2.,
            angle.cos() / 2.,
        ));
    }

    let points = circle;

    let input_points = points.clone();
    let segs = points_to_segments(&input_points);
    let pg = segments_to_point_graph(&input_points, segs);
    let eg = point_graph_to_euler_graph(&pg);
    let ec = euler_graph_to_euler_circuit(&input_points, &eg);
    let output_points: Vec<LaserdockSample> =
        interpolate_euler_circuit(&input_points, &ec, &eg, 10, &InterpolationConfig::default());

    loop {
        for chunk in output_points.chunks(SAMPLES_PER_BATCH) {
            lc.send_samples(chunk)?;
        }
    }
}
