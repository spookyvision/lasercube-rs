use std::{
    convert::TryInto,
    mem::size_of,
    ops::{Deref, DerefMut},
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use bytemuck::{cast_slice, Pod, Zeroable};
use lasy::{Blanked, IsBlank, Lerp, Position, Weight};
use log::{debug, error, info, log_enabled};
use rusb::{DeviceHandle, Direction, GlobalContext, TransferType};
use thiserror::Error;

pub const BYTES_PER_BATCH: usize = 64;
#[derive(Copy, Clone, Pod, Zeroable, Ord, PartialOrd, Eq, PartialEq, Debug, Hash)]
#[repr(C)]
pub struct XY(pub u16);

impl XY {
    pub fn flip(self) -> Self {
        XY(4095 - self.0)
    }
}

impl From<f32> for XY {
    fn from(f: f32) -> Self {
        let f = f.clamp(-1., 1.);
        XY((4095. * (f + 1.0) / 2.0) as u16)
    }
}

impl From<f64> for XY {
    fn from(f: f64) -> Self {
        let f = f.clamp(-1., 1.);
        XY((4095. * (f + 1.0) / 2.0) as u16)
    }
}

#[derive(Copy, Clone, Pod, Zeroable, Hash)]
#[repr(C)]
pub struct LaserdockSample {
    rg: u16,
    b: u16,
    x: XY,
    y: XY,
}

impl IsBlank for LaserdockSample {
    fn is_blank(&self) -> bool {
        self.rg == 0 && self.b == 0
    }
}

impl Lerp for LaserdockSample {
    type Scalar = f32;

    fn lerp(&self, dest: &Self, amt: Self::Scalar) -> Self {
        fn lerp_u16(a: u16, b: u16, amount: f32) -> u16 {
            a + ((b as f32 - a as f32) * amount).round() as u16
        }

        LaserdockSample {
            rg: lerp_u16(self.rg, dest.rg, amt),
            b: lerp_u16(self.b, dest.b, amt),
            x: XY(lerp_u16(self.x.0, dest.x.0, amt)),
            y: XY(lerp_u16(self.y.0, dest.y.0, amt)),
        }
    }
}

impl Blanked for LaserdockSample {
    fn blanked(&self) -> Self {
        let mut blank = *self;
        blank.rg = 0;
        blank.b = 0;
        blank
    }
}
impl Position for LaserdockSample {
    fn position(&self) -> [f32; 2] {
        //XY((4095. * (f + 1.0) / 2.0) as u16)
        // xy = 4095 * (f+1)/2
        fn to_f(xy: XY) -> f32 {
            (2.0 * (xy.0 as f32) - 4095.) / 4095.
        }

        [to_f(self.x), to_f(self.y)]
    }
}

impl Weight for LaserdockSample {
    fn weight(&self) -> u32 {
        0
    }
}

pub const SAMPLE_SIZE: usize = size_of::<LaserdockSample>() / size_of::<u8>();
pub const SAMPLES_PER_BATCH: usize = BYTES_PER_BATCH / SAMPLE_SIZE;

impl LaserdockSample {
    pub fn new(r: u8, g: u8, b: u8, x: f64, y: f64) -> LaserdockSample {
        Self::new_xy(r, g, b, x.into(), y.into())
    }

    pub fn new_xy(r: u8, g: u8, b: u8, x: XY, y: XY) -> LaserdockSample {
        LaserdockSample {
            rg: r as u16 | (g as u16) << 8,
            b: b as u16,
            x,
            y,
        }
    }
}

enum SetCommand {
    ClearRingBuffer = 0x8d,
    EnableOutput = 0x80,
    DacRate = 0x82,
}

enum GetCommand {
    OutputEnabled = 0x81,
    DacRate = 0x83,
    MaxDacRate = 0x84,
    MinDacRate = 0x87,
    MaxDacValue = 0x88,
    VersionMajor = 0x8b,
    VersionMinor = 0x8c,
}

struct Buf([u8; BYTES_PER_BATCH]);

impl Buf {
    fn new() -> Self {
        Buf([0; BYTES_PER_BATCH])
    }
}

impl Deref for Buf {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Buf {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<Buf> for u32 {
    fn from(buf: Buf) -> Self {
        Self::from_le_bytes((buf.0[2..6]).try_into().unwrap())
    }
}

impl From<Buf> for u8 {
    fn from(buf: Buf) -> Self {
        buf.0[2]
    }
}

#[derive(Error, Debug)]
pub enum BusError {
    #[error("incomplete write: {0} of {1} bytes")]
    IncompleteWrite(usize, usize),

    #[error("incomplete response: {0} of {1} bytes")]
    IncompleteResponse(usize, usize),

    #[error("Unexpected content: {0} instead of {1}")]
    UnexpectedContent(u8, u8),
}

pub struct LaserCube {
    device: DeviceHandle<GlobalContext>,
    control_read: u8,
    control_write: u8,
    data_write: u8,
}

impl LaserCube {
    const USB_VENDOR_ID: u16 = 0x1fc9;
    const USB_PRODUCT_ID: u16 = 0x04d8;
    const CONTROL_INTERFACE: u8 = 0;
    const DATA_INTERFACE: u8 = 1;
    const RECV_BUF_LEN: usize = 64;

    pub fn open_first() -> Result<LaserCube> {
        let device = rusb::devices()?
            .iter()
            .filter_map(|device| {
                let descriptor = device.device_descriptor().ok()?;
                if descriptor.vendor_id() == Self::USB_VENDOR_ID
                    && descriptor.product_id() == Self::USB_PRODUCT_ID
                {
                    Some(device)
                } else {
                    None
                }
            })
            .next()
            .ok_or(anyhow!("LaserCube not found"))?;

        let config_desc = device.config_descriptor(0)?;

        let mut control_read = None;
        let mut control_write = None;
        let mut data_write = None;

        let mut device = device.open()?;

        device.claim_interface(Self::CONTROL_INTERFACE)?;
        device.claim_interface(Self::DATA_INTERFACE)?;

        for interface in config_desc.interfaces() {
            for interface_desc in interface.descriptors() {
                if interface_desc.interface_number() == Self::CONTROL_INTERFACE {
                    for endpoint_desc in interface_desc.endpoint_descriptors() {
                        if endpoint_desc.direction() == Direction::In {
                            control_read = Some(endpoint_desc.address())
                        } else if endpoint_desc.direction() == Direction::Out {
                            control_write = Some(endpoint_desc.address());
                        }
                    }
                }

                if interface_desc.interface_number() == Self::DATA_INTERFACE {
                    for endpoint_desc in interface_desc.endpoint_descriptors() {
                        if endpoint_desc.transfer_type() == TransferType::Bulk {
                            device.set_alternate_setting(
                                Self::DATA_INTERFACE,
                                interface_desc.setting_number(),
                            )?;

                            data_write = Some(endpoint_desc.address());
                        }
                    }
                }
            }
        }

        let control_read = control_read.ok_or(anyhow!("control interface not found"))?;
        let control_write = control_write.ok_or(anyhow!("control interface not found"))?;
        let data_write = data_write.ok_or(anyhow!("data interface not found"))?;

        let mut laser_cube = LaserCube {
            device: device,
            control_read,
            control_write,
            data_write,
        };

        if log_enabled!(log::Level::Debug) {
            laser_cube.diagnostics()?
        }

        laser_cube.clear_ringbuffer()?;
        laser_cube.enable_output()?;
        if !laser_cube.output_enabled()? {
            return Err(anyhow!("failed to enable output"));
        } else {
            info!("Output enabled!")
        }

        Ok(laser_cube)
    }

    fn read<T: From<Buf>>(&self, command: GetCommand) -> Result<T> {
        let recv = self.write_buf(&[command as u8])?;

        Ok(recv.into())
    }

    fn write_u32(&mut self, command: SetCommand, value: u32) -> Result<()> {
        let mut buf = Vec::with_capacity(5);

        buf.push(command as u8);
        buf.extend_from_slice(&value.to_le_bytes());
        self.write_buf(&buf)?;
        Ok(())
    }

    fn write_u8(&mut self, command: SetCommand, value: u8) -> Result<()> {
        self.write_buf(&[command as u8, value])?;
        Ok(())
    }

    fn write_buf(&self, buf: &[u8]) -> Result<Buf> {
        let timeout = Duration::from_secs(1);

        let written = self
            .device
            .write_bulk(self.control_write, &buf, timeout)
            .context("write_bulk")?;

        if written != buf.len() {
            return Err(BusError::IncompleteWrite(written, buf.len()).into());
        }

        let mut recv = Buf::new();
        let read = self
            .device
            .read_bulk(self.control_read, &mut recv, timeout)
            .context("read_bulk")?;

        if read != LaserCube::RECV_BUF_LEN {
            return Err(BusError::IncompleteResponse(read, LaserCube::RECV_BUF_LEN).into());
        }

        if recv[1] != 0 {
            return Err(BusError::UnexpectedContent(recv[1], 0).into());
        }

        Ok(recv)
    }

    pub fn send_samples(&self, buf: &[LaserdockSample]) -> Result<()> {
        self.send(cast_slice(buf))
    }

    pub fn send(&self, buf: &[u8]) -> Result<()> {
        let timeout = Duration::from_secs(1);

        let written = self.device.write_bulk(self.data_write, &buf, timeout)?;

        if written != buf.len() {
            return Err(BusError::IncompleteWrite(written, buf.len()).into());
        }

        Ok(())
    }

    pub fn max_dac_rate(&self) -> Result<u32> {
        self.read::<u32>(GetCommand::MaxDacRate)
    }

    pub fn min_dac_rate(&self) -> Result<u32> {
        self.read::<u32>(GetCommand::MinDacRate)
    }

    pub fn dac_rate(&self) -> Result<u32> {
        self.read::<u32>(GetCommand::DacRate)
    }

    pub fn set_dac_rate(&mut self, rate: u32) -> Result<()> {
        let min = self.min_dac_rate()?;
        let max = self.max_dac_rate()?;
        self.write_u32(SetCommand::DacRate, rate.clamp(min, max))
    }

    pub fn clear_ringbuffer(&mut self) -> Result<()> {
        debug!("clearing ring buffer");
        self.write_u8(SetCommand::ClearRingBuffer, 0)
    }

    pub fn enable_output(&mut self) -> Result<()> {
        debug!("enabling output");
        self.write_u8(SetCommand::EnableOutput, 1)?;
        Ok(())
    }

    pub fn disable_output(&mut self) -> Result<()> {
        debug!("disabling output");
        self.write_u8(SetCommand::EnableOutput, 0)?;
        Ok(())
    }

    pub fn output_enabled(&self) -> Result<bool> {
        Ok(true)
    }

    pub fn diagnostics(&self) -> Result<()> {
        let timeout = Duration::from_secs(1);
        let device_handle = &self.device;
        let descriptor = &device_handle.device().device_descriptor()?;

        let languages = device_handle.read_languages(timeout)?;

        debug!(
            "Active configuration: {}",
            device_handle.active_configuration()?
        );
        debug!("Languages: {:?}", languages);

        if languages.len() > 0 {
            let language = languages[0];

            debug!(
                "Manufacturer: {:?}",
                device_handle
                    .read_manufacturer_string(language, &descriptor, timeout)
                    .unwrap_or("?".to_string())
            );
            debug!(
                "Product: {:?}",
                device_handle
                    .read_product_string(language, &descriptor, timeout)
                    .unwrap_or("?".to_string())
            );
            debug!(
                "Serial Number: {:?}",
                device_handle
                    .read_serial_number_string(language, &descriptor, timeout)
                    .unwrap_or("?".to_string())
            );
        }

        debug!(
            "v{}.{}",
            self.read::<u32>(GetCommand::VersionMajor)?,
            self.read::<u32>(GetCommand::VersionMinor)?
        );

        debug!("min dac rate {}", self.min_dac_rate()?);
        debug!("max dac rate {}", self.max_dac_rate()?);
        debug!("dac rate {}", self.dac_rate()?);
        debug!(
            "max dac value {}",
            self.read::<u32>(GetCommand::MaxDacValue)?
        );

        Ok(())
    }
}

impl Default for LaserCube {
    fn default() -> Self {
        Self::open_first().unwrap()
    }
}
