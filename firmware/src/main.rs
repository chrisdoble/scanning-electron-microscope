#![no_main]
#![no_std]

use common::{Error, USB_MAX_PACKET_SIZE};
use defmt::*;
use embassy_executor::Spawner;
use embassy_rp::{
    Peri, binary_info, bind_interrupts, dma,
    gpio::{Level, Output},
    interrupt::typelevel::Binding,
    pac,
    peripherals::{UART0, UART1, USB},
    uart::{self, Uart},
    usb,
};
use embassy_time::{Duration, Timer, with_timeout};
use embassy_usb::{
    UsbDevice,
    class::cdc_acm::{BufferedReceiver, CdcAcmClass, Sender, State},
    driver::EndpointError,
};
use embedded_io_async::Read;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(struct Irqs {
    UART0_IRQ => uart::InterruptHandler<UART0>;
    UART1_IRQ => uart::InterruptHandler<UART1>;
    USBCTRL_IRQ => usb::InterruptHandler<USB>;
});

// This data appears in the output of `pictool info`[1].
//
// 1: https://docs.rs/rp-binary-info/0.1.1/rp_binary_info/
#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [binary_info::EntryAddr; 3] = [
    binary_info::rp_program_name!(c"SEM controller"),
    binary_info::rp_program_description!(c"Controls my DIY SEM"),
    binary_info::rp_cargo_version!(),
];

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let peripherals = embassy_rp::init(Default::default());

    // Initialise the USB device and the CDC ACM sender/receiver.
    let (usb_device, mut sender, mut receiver) = init_usb(peripherals.USB);

    // Spawn a task to run the USB driver.
    spawner.spawn(usb_task(usb_device).unwrap());

    // Initialise the UART driver for the Edwards ADC MkII (pressure gauge).
    //
    // This is connected to an RS-232 converter, so we can read/write as usual.
    let mut uart0 = init_uart(
        peripherals.UART0,
        peripherals.PIN_16,
        peripherals.PIN_17,
        Irqs,
        peripherals.DMA_CH0,
        peripherals.DMA_CH1,
    );

    // Initialise the UART driver for the Pfeiffer TC 600 (TMP controller).
    //
    // This is connected to an RS-485 converter, so we can read/write as usual.
    let mut uart1 = init_uart(
        peripherals.UART1,
        peripherals.PIN_4,
        peripherals.PIN_5,
        Irqs,
        peripherals.DMA_CH2,
        peripherals.DMA_CH3,
    );

    // The RS-485 converter contains a driver and a receiver, both connected to
    // the same bus. To receive we want the driver disabled and the receiver
    // enabled (otherwise the driver would drive the line high while idle). To
    // transmit we want the opposite (otherwise we would receive our own
    // transmission). Set this pin high to transmit and set it low to read.
    let mut uart1_transmit_pin = Output::new(peripherals.PIN_3, Level::Low);

    // USB packets that are exactly the maximum packet size aren't processed
    // until a subsequent shorter packet is sent. We may not have a subsequent
    // packet to send but we want all packets to be sent immediately. Set the
    // buffer size to be 1 byte smaller than the maximum to avoid this problem.
    let mut command = [0u8; USB_MAX_PACKET_SIZE as usize - 1];
    let mut response = [0u8; USB_MAX_PACKET_SIZE as usize - 1];

    loop {
        // Wait for a USB host to connect.
        info!("Waiting for USB host");
        receiver.wait_connection().await;
        info!("USB host connected");

        loop {
            let result: Result<(), Error> = (async {
                // Read a command from the USB host.
                let command_length = read_command(&mut receiver, &mut command).await?;

                // Change the last character of the command to a carriage
                // return, as is expected by both the ADC and the TMP.
                command[command_length - 1] = b'\r';

                // Split the command into destination and command.
                let destination = &command[..3];
                let command = &command[4..command_length];

                let mut response_length: usize;
                match destination {
                    b"ADC" => {
                        write_command(command, "ADC", &mut uart0).await?;
                        response_length = read_response(&mut response, "ADC", &mut uart0).await?;
                    }
                    b"TMP" => {
                        write_rs_485_command(
                            command,
                            "TMP",
                            &mut uart1_transmit_pin,
                            &mut uart1,
                            &pac::UART1,
                        )
                        .await?;
                        response_length = read_response(&mut response, "TMP", &mut uart1).await?;
                    }
                    _ => return Err(Error::UnknownDestination),
                }

                // Our response to the USB host must be terminated by a
                // carriage return followed by a newline (\r\n). Both the ADC
                // and the TMP terminate their responses with a carriage return
                // (\r) so we just need to add a newline. Ensure there's enough
                // space in the buffer to do that, then do it.
                if response_length == response.len() {
                    return Err(Error::ResponseTooLong);
                }
                response[response_length] = b'\n';
                response_length += 1;

                // Send the response to the USB host.
                send_response(&response[..response_length], &mut sender).await
            })
            .await;

            match result {
                Ok(()) => {}
                Err(Error::Disconnected) => {
                    info!("USB host disconnected");
                    break;
                }
                Err(e) => {
                    error!("Error: {}", e);
                    send_response(e.to_response(), &mut sender)
                        .await
                        .expect("couldn't send error response");
                }
            }
        }
    }
}

/// Initialises a USB device and a CDC ACM sender/receiver.
///
/// # Panics
///
/// Panics if called more than once.
fn init_usb<'a>(
    usb: Peri<'static, USB>,
) -> (
    UsbDevice<'static, usb::Driver<'static, USB>>,
    Sender<'static, usb::Driver<'static, USB>>,
    BufferedReceiver<'static, usb::Driver<'static, USB>>,
) {
    let driver = usb::Driver::new(usb, Irqs);
    let config = {
        // VID/PID taken from https://pid.codes/1209/0001/
        let mut config = embassy_usb::Config::new(0x1209, 0x0001);
        config.manufacturer = Some("Chris Doble");
        config.max_packet_size_0 = USB_MAX_PACKET_SIZE;
        config.product = Some("SEM controller");
        config
    };
    let mut builder = {
        // 256 bytes should be more than enough to contain the descriptors.
        static CONFIG_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
        static BOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();

        // The control buffer length must be equal to `config.max_packet_size_0`.
        static CONTROL_BUFFER: StaticCell<[u8; USB_MAX_PACKET_SIZE as usize]> = StaticCell::new();

        embassy_usb::Builder::new(
            driver,
            config,
            CONFIG_DESCRIPTOR.init([0u8; 256]),
            BOS_DESCRIPTOR.init([0u8; 256]),
            // No Microsoft OS descriptors.
            &mut [],
            CONTROL_BUFFER.init([0; USB_MAX_PACKET_SIZE as usize]),
        )
    };
    let cdc_acm = {
        static STATE: StaticCell<State> = StaticCell::new();
        CdcAcmClass::new(
            &mut builder,
            STATE.init(State::new()),
            USB_MAX_PACKET_SIZE as u16,
        )
    };
    let (sender, receiver) = cdc_acm.split();
    static RECEIVE_BUFFER: StaticCell<[u8; USB_MAX_PACKET_SIZE as usize]> = StaticCell::new();
    (
        builder.build(),
        sender,
        receiver.into_buffered(RECEIVE_BUFFER.init([0u8; USB_MAX_PACKET_SIZE as usize])),
    )
}

/// Runs the given `UsbDevice` forever.
#[embassy_executor::task]
async fn usb_task(mut usb_device: UsbDevice<'static, usb::Driver<'static, USB>>) {
    usb_device.run().await
}

/// Initialises a UART driver using the given peripherals.
///
/// The driver communicates at 9600 baud with 1 stop bit, 8 data bits, and no
/// parity bit (as used by the Edwards ADC MkII and the Pfeiffer TC 600).
fn init_uart<T: uart::Instance>(
    uart: Peri<'static, T>,
    tx_pin: Peri<'static, impl uart::TxPin<T>>,
    rx_pin: Peri<'static, impl uart::RxPin<T>>,
    irq: impl Binding<T::Interrupt, uart::InterruptHandler<T>>,
    tx_dma: Peri<'static, impl dma::Channel>,
    rx_dma: Peri<'static, impl dma::Channel>,
) -> Uart<'static, uart::Async> {
    let config = {
        // The only default `Config` value that doesn't match is the baud rate.
        let mut config = uart::Config::default();
        config.baudrate = 9600;
        config
    };

    Uart::new(uart, tx_pin, rx_pin, irq, tx_dma, rx_dma, config)
}

/// Reads a command from the USB host.
///
/// A command may be terminated by a newline (\n), a carriage return (\r), or a
/// carriage return followed by a newline (\r\n). If the last is used, only the
/// carriage return (\r) is included in `command` — the newline (\n) is ignored.
///
/// If the command is successfully received and valid, its length is returned.
async fn read_command(
    receiver: &mut BufferedReceiver<'static, usb::Driver<'static, USB>>,
    command: &mut [u8],
) -> Result<usize, Error> {
    let mut char = [0u8; 1];
    let mut length = 0;

    // Read one character at a time until we get to the end.
    loop {
        // Use a long timeout to support a human typing the command.
        match with_timeout(Duration::from_secs(2), receiver.read(&mut char)).await {
            Ok(Ok(_)) => {
                command[length] = char[0];

                // If the first character is a newline, it's likely the end of
                // the \r\n terminator of the previous command. Ignore it.
                if char[0] == b'\n' && length == 0 {
                    continue;
                }

                length += 1;

                if length == command.len() {
                    return Err(Error::CommandTooLong);
                }
            }
            Ok(Err(EndpointError::Disabled)) => return Err(Error::Disconnected),
            Ok(Err(e)) => {
                error!("USB error: {}", e);
                return Err(Error::Unknown);
            }
            Err(_) => {
                if length == 0 {
                    // A command isn't being sent yet. Keep waiting.
                    continue;
                } else {
                    return Err(Error::CommandTimeout);
                }
            }
        }

        if char[0] == b'\n' || char[0] == b'\r' {
            break;
        }
    }

    if length < 6 {
        return Err(Error::CommandTooShort);
    }

    if command[3] != b':' {
        return Err(Error::CommandMissingDestination);
    }

    info!("Received command: {=[u8]:a}", command[..length]);
    Ok(length)
}

/// Writes a command to a UART driver.
async fn write_command(
    command: &[u8],
    destination: &'static str,
    uart: &mut Uart<'_, uart::Async>,
) -> Result<(), Error> {
    info!("Sending command to {}: {=[u8]:a}", destination, command);
    match uart.write(command).await {
        Ok(_) => Ok(()),
        Err(e) => {
            error!("UART error: {}", e);
            return Err(Error::Unknown);
        }
    }
}

/// Writes a command to a UART driver and toggles the RS-485 transmit pin.
async fn write_rs_485_command(
    command: &[u8],
    destination: &'static str,
    transmit_pin: &mut Output<'_>,
    uart_driver: &mut Uart<'_, uart::Async>,
    uart_pac: &pac::uart::Uart,
) -> Result<(), Error> {
    transmit_pin.set_high();

    // We must set the transmit pin low after writing, even on failure. Else the
    // driver in the UART to RS-485 converter will hold the bus high and we
    // won't be able to receive anything any more. Hold the result until then.
    let result = write_command(command, destination, uart_driver).await;

    // Wait until the transmission has truly finished. If we set the transmit
    // pin low too early we might accidentally truncate the end of the command.
    while uart_pac.uartfr().read().busy() {
        // We don't want to wait too long per iteration or we might miss the
        // response. 100 µs is around the time it takes to transmit 1 bit.
        Timer::after_micros(100).await;
    }

    transmit_pin.set_low();
    result
}

/// Reads a response from a UART driver.
///
/// A carriage return (\r) signals the end of the response.
///
/// If the command is successfully received and valid, its length is returned.
async fn read_response(
    response: &mut [u8],
    sender: &'static str,
    uart: &mut Uart<'_, uart::Async>,
) -> Result<usize, Error> {
    let mut char = [0u8; 1];
    let mut length = 0;

    loop {
        // From experimentation, the ADC can take up to ~300 ms to respond. Set
        // the timeout duration to 400 ms to give ourselves a bit of a buffer.
        let duration = Duration::from_millis(400);
        match with_timeout(duration, uart.read(&mut char)).await {
            Ok(Ok(_)) => {
                response[length] = char[0];
                length += 1;

                if length == response.len() {
                    return Err(Error::ResponseTooLong);
                }
            }
            Ok(Err(e)) => {
                error!("UART error: {}", e);
                return Err(Error::Unknown);
            }
            Err(_) => return Err(Error::ResponseTimeout),
        }

        if response[length - 1] == b'\r' {
            break;
        }
    }

    info!(
        "Received response from {}: {=[u8]:a}",
        sender,
        response[..length]
    );
    Ok(length)
}

/// Sends a response to the USB host.
async fn send_response(
    response: &[u8],
    sender: &mut Sender<'static, usb::Driver<'static, USB>>,
) -> Result<(), Error> {
    info!("Sending response: {=[u8]:a}", response);
    match sender.write_packet(response).await {
        Ok(()) => Ok(()),
        Err(EndpointError::Disabled) => Err(Error::Disconnected),
        Err(e) => {
            error!("USB error: {}", e);
            return Err(Error::Unknown);
        }
    }
}
