use std::{f64::consts::PI, mem::size_of};

use anyhow::Result;
use bytemuck::cast_slice;
use lasercube::{LaserCube, LaserdockSample, SAMPLES_PER_BATCH, SAMPLE_SIZE, XY};

use lasy::{
    euler_graph_to_euler_circuit, interpolate_euler_circuit, point_graph_to_euler_graph,
    points_to_segments, segments_to_point_graph, InterpolationConfig,
};
use log::debug;
fn main() -> Result<()> {
    pretty_env_logger::init();

    let mut lc = LaserCube::open_first()?;
    lc.set_dac_rate(20000)?;

    const NUM_POINTS: usize = 80;
    let mut points: Vec<LaserdockSample> = Vec::with_capacity(NUM_POINTS + SAMPLES_PER_BATCH);
    debug!(
        "buffer len in samples: {}, bytes: {}",
        points.capacity(),
        points.capacity() * SAMPLE_SIZE
    );
    let mut y = 1.0;
    let mut dir = true;
    for i in 0..points.capacity() {
        let invert = if dir { 1.0 } else { -1.0 };
        let x = ((i % 20) as f64 / 10. - 1.0) * invert;
        let r = ((x + 1.0) * 150.) as u8;

        if i % 20 == 0 {
            println!("{i} - empty");
            points.push(LaserdockSample::new(0, 0, 0, x, y));
        }

        println!("{i} - data {x}:{y}");
        points.push(LaserdockSample::new(r, 0, 10, x, y));

        if i % 20 == 19 {
            println!("{i} - empty");
            points.push(LaserdockSample::new(0, 0, 0, x, y));
            y -= 0.2;
            dir = !dir;
        }
    }

    // let mut points = vec![];
    // let p2s = [(0, 0), (2000, 0), (2000, 2000), (2000, 2000), (0, 0)];
    // let p2s = [(0, 0), (2000, 0), (2000, 0)];

    // for add in [0] {
    //     for (x, y) in &p2s {
    //         points.push(LaserdockSample::new_xy(0, 1, 2, XY(x + add), XY(y + add)));
    //     }

    //     points.push(LaserdockSample::new_xy(
    //         0,
    //         0,
    //         0,
    //         XY(p2s[p2s.len() - 1].0 * 2 + add),
    //         XY(p2s[p2s.len() - 1].1 * 2 + add),
    //     ));
    // }
    let input_points = points.clone();
    let segs = points_to_segments(&input_points);
    let pg = segments_to_point_graph(&input_points, segs);
    let eg = point_graph_to_euler_graph(&pg);
    let ec = euler_graph_to_euler_circuit(&input_points, &eg);
    let output_points: Vec<LaserdockSample> =
        interpolate_euler_circuit(&input_points, &ec, &eg, 20, &InterpolationConfig::default());

    loop {
        for chunk in points.chunks(SAMPLES_PER_BATCH) {
            lc.send_samples(chunk)?;
        }
    }

    Ok(())
}
