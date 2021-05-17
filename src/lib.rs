use anyhow::{anyhow, Context, Result};
use libusb::{self, Device, DeviceDescriptor, DeviceHandle, Devices, Direction, TransferType};
use std::{
    convert::TryInto,
    ops::{Deref, DerefMut},
    time::Duration,
};
use thiserror::Error;

use log::{debug, error, info, log_enabled};

use bytemuck::{cast_slice, Pod, Zeroable};

#[derive(Copy, Clone, Pod, Zeroable)]
#[repr(C)]
struct XY(u16);

impl XY {
    fn flip(self) -> Self {
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

#[derive(Copy, Clone, Pod, Zeroable)]
#[repr(C)]
pub struct LaserdockSample {
    rg: u16,
    b: u16,
    x: XY,
    y: XY,
}

impl LaserdockSample {
    pub fn new(r: u8, g: u8, b: u8, x: f64, y: f64) -> LaserdockSample {
        LaserdockSample {
            rg: r as u16 | (g as u16) << 8,
            b: b as u16,
            x: x.into(),
            y: y.into(),
        }
    }
}
pub struct LaserCube<'usb> {
    control_handle: DeviceHandle<'usb>,
    data_handle: DeviceHandle<'usb>,
    control_read: Endpoint,
    control_write: Endpoint,
    data_write: Endpoint,
    descriptor: DeviceDescriptor,
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

struct Buf([u8; 64]);

impl Buf {
    fn new() -> Self {
        Buf([0; 64])
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

impl<'usb> LaserCube<'usb> {
    const USB_VENDOR_ID: u16 = 0x1fc9;
    const USB_PRODUCT_ID: u16 = 0x04d8;

    const RECV_BUF_LEN: usize = 64;
    pub fn usb_devices<'b, 'a: 'b>(
        it: Devices<'a, 'b>,
    ) -> impl Iterator<Item = (Device<'b>, DeviceDescriptor)> + 'b {
        it.filter_map(|device| {
            if let Ok(device_desc) = device.device_descriptor() {
                if device_desc.vendor_id() == Self::USB_VENDOR_ID
                    && device_desc.product_id() == Self::USB_PRODUCT_ID
                {
                    return Some((device, device_desc));
                }
            }
            None
        })
    }

    fn read<T: From<Buf>>(&mut self, command: GetCommand) -> Result<T> {
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

    fn write_buf(&mut self, buf: &[u8]) -> Result<Buf> {
        let timeout = Duration::from_secs(1);

        let written = self
            .control_handle
            .write_bulk(self.control_write.address, &buf, timeout)
            .context("write_bulk")?;

        if written != buf.len() {
            return Err(BusError::IncompleteWrite(written, buf.len()).into());
        }

        let mut recv = Buf::new();
        let read = self
            .control_handle
            .read_bulk(self.control_read.address, &mut recv, timeout)
            .context("read_bulk")?;

        if read != LaserCube::RECV_BUF_LEN {
            return Err(BusError::IncompleteResponse(read, LaserCube::RECV_BUF_LEN).into());
        }

        if recv[1] != 0 {
            return Err(BusError::UnexpectedContent(recv[1], 0).into());
        }

        Ok(recv)
    }

    pub fn max_dac_rate(&mut self) -> Result<u32> {
        self.read::<u32>(GetCommand::MaxDacRate)
    }

    pub fn min_dac_rate(&mut self) -> Result<u32> {
        self.read::<u32>(GetCommand::MinDacRate)
    }

    pub fn set_dac_rate(&mut self, rate: u32) -> Result<()> {
        let min = self.min_dac_rate()?;
        let max = self.max_dac_rate()?;
        self.write_u32(SetCommand::DacRate, rate.clamp(min, max))
    }

    pub fn send_samples(&mut self, buf: &[LaserdockSample]) -> Result<()> {
        self.send(cast_slice(buf))
    }

    pub fn send(&mut self, buf: &[u8]) -> Result<()> {
        let timeout = Duration::from_secs(1);

        let written = self
            .data_handle
            .write_bulk(self.data_write.address, &buf, timeout)?;

        if written != buf.len() {
            return Err(BusError::IncompleteWrite(written, buf.len()).into());
        }

        Ok(())
    }

    pub fn dd_from_context<'b, 'a: 'b>(
        it: Devices<'a, 'b>,
    ) -> Result<(Device<'b>, DeviceDescriptor)> {
        Self::usb_devices(it)
            .next()
            .ok_or(anyhow!("could not find a Las0r"))
    }

    pub fn new(mut device: Device<'usb>, descriptor: DeviceDescriptor) -> Result<Self> {
        let mut control_handle = device.open()?;

        let mut data_handle = device.open()?;

        //find_bulk_endpoints(&mut device, &descriptor);
        let control_read = find_bulk_endpoint(&mut device, &descriptor, 0, Direction::In)
            .ok_or(anyhow!("did not find control endpoint"))?;

        let control_write = find_bulk_endpoint(&mut device, &descriptor, 0, Direction::Out)
            .ok_or(anyhow!("did not find control endpoint"))?;

        configure_endpoint(&mut control_handle, &control_read)?;
        configure_endpoint(&mut control_handle, &control_write)?;

        let data_write = find_bulk_endpoint(&mut device, &descriptor, 1, Direction::Out)
            .ok_or(anyhow!("did not find data endpoint"))?;

        configure_endpoint(&mut data_handle, &data_write)?;
        //data_handle.set_alternate_setting(1, 1)?;

        let mut laser_cube = Self {
            control_handle,
            data_handle,
            control_read,
            control_write,
            data_write,
            descriptor,
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

    pub fn clear_ringbuffer(&mut self) -> Result<()> {
        debug!("clearing ring buffer");
        self.write_u8(SetCommand::ClearRingBuffer, 0)
    }

    pub fn enable_output(&mut self) -> Result<()> {
        debug!("enabling output");
        self.write_u8(SetCommand::EnableOutput, 1)?;
        Ok(())
    }

    pub fn output_enabled(&mut self) -> Result<(bool)> {
        Ok(true)
    }

    pub fn disable_output(&mut self) -> Result<()> {
        debug!("disabling output");
        self.write_u8(SetCommand::EnableOutput, 0)?;
        Ok(())
    }

    pub fn diagnostics(&mut self) -> Result<()> {
        let timeout = Duration::from_secs(1);
        let device_handle = &self.control_handle;
        let descriptor = &self.descriptor;

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
        debug!("dac rate {}", self.read::<u32>(GetCommand::DacRate)?);
        debug!(
            "max dac value {}",
            self.read::<u32>(GetCommand::MaxDacValue)?
        );

        Ok(())
    }
}

fn find_bulk_endpoint(
    device: &mut libusb::Device,
    device_desc: &libusb::DeviceDescriptor,
    interface_number: u8,
    direction: Direction,
) -> Option<Endpoint> {
    for n in 0..device_desc.num_configurations() {
        let config_desc = match device.config_descriptor(n) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for interface in config_desc.interfaces() {
            for interface_desc in interface.descriptors() {
                for endpoint_desc in interface_desc.endpoint_descriptors() {
                    if endpoint_desc.direction() == direction
                        && endpoint_desc.transfer_type() == TransferType::Bulk
                        && interface_desc.interface_number() == interface_number
                    {
                        return Some(Endpoint {
                            config: config_desc.number(),
                            iface: interface_desc.interface_number(),
                            setting: interface_desc.setting_number(),
                            address: endpoint_desc.address(),
                        });
                    }
                }
            }
        }
    }

    None
}

// unused; just here to list everything a device has to offer
fn find_bulk_endpoints(
    device: &mut libusb::Device,
    device_desc: &libusb::DeviceDescriptor,
) -> Option<Endpoint> {
    for n in 0..device_desc.num_configurations() {
        let config_desc = match device.config_descriptor(n) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for interface in config_desc.interfaces() {
            for interface_desc in interface.descriptors() {
                for endpoint_desc in interface_desc.endpoint_descriptors() {
                    if endpoint_desc.transfer_type() == TransferType::Bulk {
                        let p = Endpoint {
                            config: config_desc.number(),
                            iface: interface_desc.interface_number(),
                            setting: interface_desc.setting_number(),
                            address: endpoint_desc.address(),
                        };

                        println!("found {:#?} {:#?}", p, endpoint_desc.direction());
                    }
                }
            }
        }
    }

    None
}

#[derive(Debug)]
struct Endpoint {
    config: u8,
    iface: u8,
    setting: u8,
    address: u8,
}

fn configure_endpoint<'a>(
    handle: &'a mut libusb::DeviceHandle,
    endpoint: &Endpoint,
) -> libusb::Result<()> {
    debug!("configure endpoint {:#?}", endpoint);
    //handle.reset()?;
    handle.set_active_configuration(endpoint.config)?;
    handle.claim_interface(endpoint.iface)?;
    handle.set_alternate_setting(endpoint.iface, endpoint.setting)?;
    Ok(())
}
