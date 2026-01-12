use common::{ControllerError, USB_MAX_PACKET_SIZE};
use log::*;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The destination for a command sent to the controller.
#[derive(Debug)]
pub enum Destination {
    /// The Edwards ADC MkII pressure gauge controller.
    ADC,

    // The Pfeiffer TC 600 TMP controller.
    TMP,
}

pub struct Controller {
    port: Arc<Mutex<Box<dyn serialport::SerialPort>>>,
}

impl Controller {
    pub fn new(path: &str) -> Result<Controller, serialport::Error> {
        Ok(Controller {
            port: Arc::new(Mutex::new(
                serialport::new(path, 9600)
                    // From experimentation, the ADC can take up to ~300 ms to
                    // respond. Set the timeout to 500 ms to give us a buffer.
                    .timeout(Duration::from_millis(500))
                    .open()?,
            )),
        })
    }

    /// Sends a command to a particular destination and returns the response.
    ///
    /// `command` doesn't need to include the destination prefix or the `\r\n`
    /// terminator. The response won't include the `\r\n` terminator.
    pub async fn send_command(
        &self,
        destination: Destination,
        command: &str,
    ) -> Result<String, ControllerError> {
        let command = format!("{:?}:{}\r\n", destination, command);
        let port = Arc::clone(&self.port);

        let response = tokio::task::spawn_blocking(move || {
            // The controller can only handle one command at a time, so acquire
            // the mutex to ensure no other commands are sent until we're done.
            let mut port = port.lock().map_err(|e| {
                error!("failed to acquire mutex to send controller command: {}", e);
                ControllerError::Unknown
            })?;

            // Send the command.
            debug!("Sending command to controller: {:?}", command);
            port.write_all(command.as_bytes()).map_err(|e| {
                error!("failed to send controller command: {}", e);
                ControllerError::Unknown
            })?;

            // Read the response until we see the \r\n terminator.
            let mut response: Vec<u8> = Vec::new();
            let mut chunk = [0u8; USB_MAX_PACKET_SIZE as usize];

            // Its safe to continually read chunks (i.e. we won't read past the
            // terminator and into another response) because the controller only
            // handles one command at a time so there will only be one response.
            while !response.ends_with(b"\r\n") {
                let n = port.read(&mut chunk).map_err(|e| {
                    error!("failed to read controller response: {}", e);
                    ControllerError::Unknown
                })?;
                response.extend_from_slice(&chunk[..n]);
            }

            // Remove the \r\n terminator.
            response.truncate(response.len() - 2);

            let response =
                String::from_utf8(response).map_err(|_| ControllerError::ResponseNotUtf8)?;
            debug!("Received response from controller: {:?}", response);
            Ok(response) as Result<String, ControllerError>
        })
        .await
        .map_err(|e| {
            error!(
                "failed to spawn blocking task to send controller command: {}",
                e
            );
            ControllerError::Unknown
        })??;

        if response.starts_with("ERR:") {
            // Parse the error string after "ERR:".
            return Err(ControllerError::from_str(&response[4..])?);
        }

        Ok(response)
    }
}
