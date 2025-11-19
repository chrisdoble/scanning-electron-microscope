#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Driver, InterruptHandler};
use embassy_rp::{Peri, bind_interrupts};
use embassy_usb::UsbDevice;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::driver::EndpointError;
use panic_probe as _;
use static_cell::StaticCell;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<USB>;
});

// The maximum USB packet size that we can read or write. For full-speed devices
// (like the Raspberry Pi Pico 2), max_packet_size must be 8, 16, 32, or 64[1].
//
// 1: https://docs.embassy.dev/embassy-usb/git/default/class/cdc_acm/struct.CdcAcmClass.html#method.new
const MAX_PACKET_SIZE: u16 = 64;

#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [embassy_rp::binary_info::EntryAddr; 3] = [
    embassy_rp::binary_info::rp_program_name!(c"SEM controller"),
    embassy_rp::binary_info::rp_program_description!(c"Controls my DIY SEM"),
    embassy_rp::binary_info::rp_cargo_version!(),
];

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let (mut cdc_acm, usb_device) = init_usb(p.USB);

    spawner.spawn(usb_task(usb_device)).unwrap();

    loop {
        cdc_acm.wait_connection().await;
        let _ = echo(&mut cdc_acm).await;
    }
}

type StaticUsbDriver = Driver<'static, USB>;
type StaticCdcAcmClass = CdcAcmClass<'static, StaticUsbDriver>;
type StaticUsbDevice = UsbDevice<'static, StaticUsbDriver>;

fn init_usb(usb: Peri<'static, USB>) -> (StaticCdcAcmClass, StaticUsbDevice) {
    let driver = Driver::new(usb, Irqs);
    let config = {
        // VID/PID taken from https://pid.codes/1209/0001/
        let mut config = embassy_usb::Config::new(0x1209, 0x0001);
        config.manufacturer = Some("Chris Doble");
        config.product = Some("SEM controller");
        config
    };
    let mut builder = {
        // 256 bytes should be more than enough to contain the descriptors.
        static CONFIG_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
        static BOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();

        // The control buffer length must be equal to `config.max_packet_size_0`
        // which defaults to 64. If we change it, this needs to be updated.
        static CONTROL_BUFFER: StaticCell<[u8; 64]> = StaticCell::new();

        embassy_usb::Builder::new(
            driver,
            config,
            CONFIG_DESCRIPTOR.init([0; 256]),
            BOS_DESCRIPTOR.init([0; 256]),
            &mut [], // No Microsoft OS descriptors
            CONTROL_BUFFER.init([0; 64]),
        )
    };
    let cdc_acm = {
        static STATE: StaticCell<State> = StaticCell::new();
        CdcAcmClass::new(&mut builder, STATE.init(State::new()), MAX_PACKET_SIZE)
    };
    (cdc_acm, builder.build())
}

#[embassy_executor::task]
async fn usb_task(mut usb_device: StaticUsbDevice) {
    usb_device.run().await
}

struct Disconnected {}

impl From<EndpointError> for Disconnected {
    fn from(endpoint_error: EndpointError) -> Self {
        match endpoint_error {
            EndpointError::BufferOverflow => panic!("USB buffer overflow"),
            EndpointError::Disabled => Disconnected {},
        }
    }
}

async fn echo(cdc_acm: &mut StaticCdcAcmClass) -> Result<(), Disconnected> {
    let mut buffer: [u8; MAX_PACKET_SIZE as usize] = [0; MAX_PACKET_SIZE as usize];
    loop {
        let n = cdc_acm.read_packet(&mut buffer).await?;
        cdc_acm.write_packet(&buffer[..n]).await?;
    }
}
