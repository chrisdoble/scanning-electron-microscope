#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_rp::{
    Peri, binary_info, bind_interrupts, dma,
    interrupt::typelevel::Binding,
    peripherals,
    uart::{self, Uart},
    usb,
};
use embassy_time::{Duration, with_timeout};
use embassy_usb::{
    UsbDevice,
    class::cdc_acm::{BufferedReceiver, CdcAcmClass, Sender, State},
    driver::EndpointError,
};
use embedded_io_async::Read;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    UART0_IRQ => uart::InterruptHandler<peripherals::UART0>;
    USBCTRL_IRQ => usb::InterruptHandler<peripherals::USB>;
});

// This data appears in the output of `pictool info`[1].
//
// 2: https://docs.rs/rp-binary-info/0.1.1/rp_binary_info/
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [binary_info::EntryAddr; 3] = [
    binary_info::rp_program_name!(c"SEM controller"),
    binary_info::rp_program_description!(c"Controls my DIY SEM"),
    binary_info::rp_cargo_version!(),
];

// The duration for which we'll wait before considering an operation timed out.
const TIMEOUT_DURATION: Duration = Duration::from_secs(3);

// The maximum USB packet size that we can read or write. For full-speed devices
// (like the Raspberry Pi Pico 2), this must be 8, 16, 32, or 64[1].
//
// 1: https://docs.embassy.dev/embassy-usb/git/default/class/cdc_acm/struct.CdcAcmClass.html#method.new
const USB_MAX_PACKET_SIZE: u16 = 64;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    let mut uart = init_uart(p.UART0, p.PIN_16, p.PIN_17, Irqs, p.DMA_CH0, p.DMA_CH1);
    let (mut sender, mut receiver, usb_device) = init_usb(p.USB);

    spawner.spawn(usb_task(usb_device).unwrap());

    let mut command = [0u8; 16];
    let mut response = [0u8; 16];

    loop {
        // Wait for a computer to connect.
        receiver.wait_connection().await;

        // Read a command from the computer, send it to the ADC, read its
        // response, and send that back to the computer until it disconnects.
        loop {
            let result: Result<(), UsbError> = (async {
                let command_length = read_command(&mut receiver, &mut command).await?;
                command[command_length - 1] = b'\r';
                send_command(&mut uart, &command[..command_length]).await?;
                let response_length = read_response(&mut uart, &mut response).await?;
                send_response(&mut sender, &response[..response_length]).await
            })
            .await;

            match result {
                Ok(()) => {}
                Err(UsbError::Disconnected) => break,
                Err(UsbError::Generic(error_message)) => {
                    send_response(&mut sender, error_message.as_bytes())
                        .await
                        .unwrap()
                }
            }
        }
    }
}

fn init_uart<'a, T: uart::Instance>(
    uart: Peri<'a, T>,
    tx_pin: Peri<'a, impl uart::TxPin<T>>,
    rx_pin: Peri<'a, impl uart::RxPin<T>>,
    irq: impl Binding<T::Interrupt, uart::InterruptHandler<T>>,
    tx_dma: Peri<'a, impl dma::Channel>,
    rx_dma: Peri<'a, impl dma::Channel>,
) -> uart::Uart<'a, uart::Async> {
    let config = {
        // Edwards' ADC MkII communicates via RS-232 at a rate of 9600 baud with
        // 1 stop bit, 8 data bits, and no parity bit. The only default value of
        // the UART configuration that doesn't match is the baud rate.
        let mut config = uart::Config::default();
        config.baudrate = 9600;
        config
    };

    uart::Uart::new(uart, tx_pin, rx_pin, irq, tx_dma, rx_dma, config)
}

type StaticBufferedReceiver = BufferedReceiver<'static, StaticDriver>;
type StaticDriver = usb::Driver<'static, peripherals::USB>;
type StaticSender = Sender<'static, StaticDriver>;
type StaticUsbDevice = UsbDevice<'static, StaticDriver>;

fn init_usb(
    usb: Peri<'static, peripherals::USB>,
) -> (StaticSender, StaticBufferedReceiver, StaticUsbDevice) {
    let driver = usb::Driver::new(usb, Irqs);
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
        CdcAcmClass::new(&mut builder, STATE.init(State::new()), USB_MAX_PACKET_SIZE)
    };
    static RECEIVE_BUFFER: StaticCell<[u8; USB_MAX_PACKET_SIZE as usize]> = StaticCell::new();
    let (sender, receiver) = cdc_acm.split();
    (
        sender,
        receiver.into_buffered(RECEIVE_BUFFER.init([0u8; USB_MAX_PACKET_SIZE as usize])),
        builder.build(),
    )
}

#[embassy_executor::task]
async fn usb_task(mut usb_device: StaticUsbDevice) {
    usb_device.run().await
}

#[derive(Debug)]
enum UsbError {
    /// The computer disconnected.
    Disconnected,

    /// Any other error.
    Generic(&'static str),
}

impl From<&'static str> for UsbError {
    fn from(val: &'static str) -> Self {
        UsbError::Generic(val)
    }
}

/// Reads a command from the computer via USB.
///
/// A newline signals the end of the command. Returns the command length.
async fn read_command(
    buffered_receiver: &mut StaticBufferedReceiver,
    command: &mut [u8],
) -> Result<usize, UsbError> {
    let mut char = [0u8; 1];
    let mut length = 0;

    loop {
        match with_timeout(TIMEOUT_DURATION, buffered_receiver.read(&mut char)).await {
            Ok(Ok(_)) => {
                command[length] = char[0];
                length += 1;
            }
            Ok(Err(EndpointError::Disabled)) => return Err(UsbError::Disconnected),
            Ok(Err(_)) => {
                return Err(UsbError::Generic(
                    "Error while reading command from computer via USB\n",
                ));
            }
            Err(_) => {
                return Err(UsbError::Generic(
                    "Timeout while reading command from computer via USB\n",
                ));
            }
        }

        if command[length - 1] == b'\n' {
            return Ok(length);
        }
    }
}

/// Sends a command to the ADC via UART.
async fn send_command(
    uart: &mut Uart<'_, uart::Async>,
    command: &[u8],
) -> Result<(), &'static str> {
    match uart.write(command).await {
        Ok(_) => Ok(()),
        Err(_) => Err("Error while sending command to ADC via UART\n"),
    }
}

/// Reads a response from the ADC via UART.
///
/// A carriage return signals the end of the response. Returns the response length.
async fn read_response(
    uart: &mut Uart<'_, uart::Async>,
    response: &mut [u8],
) -> Result<usize, &'static str> {
    let mut char = [0u8; 1];
    let mut length = 0;

    loop {
        match with_timeout(TIMEOUT_DURATION, uart.read(&mut char)).await {
            Ok(Ok(_)) => {
                response[length] = char[0];
                length += 1;
            }
            Ok(Err(_)) => return Err("Error while reading response from ADC via UART\n"),
            Err(_) => return Err("Timeout while reading response from ADC via UART\n"),
        }

        if response[length - 1] == b'\r' {
            // Replace the carriage return with a newline so it prints better.
            response[length - 1] = b'\n';
            return Ok(length);
        }
    }
}

/// Sends a response to the computer via USB.
async fn send_response(sender: &mut StaticSender, response: &[u8]) -> Result<(), UsbError> {
    match sender.write_packet(response).await {
        Ok(()) => Ok(()),
        Err(EndpointError::Disabled) => Err(UsbError::Disconnected),
        Err(_) => Err(UsbError::Generic(
            "Error while sending response to computer via USB\n",
        )),
    }
}
