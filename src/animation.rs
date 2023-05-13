use std::{thread::sleep, time::Duration};

use crate::{LaserCube, LaserdockSample};

pub struct Frame {
    points: Vec<LaserdockSample>,
}

impl Frame {
    pub fn new(points: Vec<LaserdockSample>) -> Self {
        Self { points }
    }

    pub fn draw(&self, device: &LaserCube) -> anyhow::Result<()> {
        device.send_samples(&self.points)
    }
}

pub struct Animation {
    frames: Vec<Frame>,
    delay_ms: u64,
}

impl Animation {
    pub fn new(mut frames: Vec<Frame>, delay_ms: u64) -> Self {
        let num_frames = frames.len();
        if num_frames > 1 {
            for i in 0..=num_frames {
                let next = &frames[(i + 1) % num_frames];
                let mut next_start = next.points[0];
                next_start.rg = 0;
                next_start.b = 0;

                let cur = &mut frames[i % num_frames];
                let cur_end = *cur.points.last().unwrap();
                let fixup_points = 2;
                for _ in 0..fixup_points {
                    cur.points.push(cur_end)
                }
                for _ in 0..fixup_points {
                    cur.points.push(next_start)
                }
            }
        }

        Self { frames, delay_ms }
    }

    pub fn play(&self, device: &LaserCube) -> anyhow::Result<()> {
        let sleep_dur = Duration::from_millis(self.delay_ms);
        for frame in &self.frames {
            frame.draw(device)?;
            sleep(sleep_dur);
        }
        Ok(())
    }
}
