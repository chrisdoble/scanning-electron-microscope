use common::{Error, USB_MAX_PACKET_SIZE};
use log::*;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

    pub async fn send_command(&self, command: &[u8], response: &mut [u8]) -> Result<usize, Error> {
        let command = command.to_vec();
        let port = Arc::clone(&self.port);

        let result = tokio::task::spawn_blocking(move || {
            // The controller can only handle one command at a time, so acquire
            // the mutex to ensure no other commands are sent until we're done.
            let mut port = port.lock().map_err(|e| {
                error!("failed to acquire mutex to send controller command: {}", e);
                Error::Unknown
            })?;

            // Send the command.
            debug!(
                "Sending command to controller: {:?}",
                str::from_utf8(&*command).expect("controller command isn't valid utf-8")
            );
            port.write_all(&command).map_err(|e| {
                error!("failed to send controller command: {}", e);
                Error::Unknown
            })?;

            // Read the response until we see the \r\n terminator.
            let mut response: Vec<u8> = Vec::new();
            let mut chunk = [0u8; USB_MAX_PACKET_SIZE as usize];

            while !response.ends_with(b"\r\n") {
                let n = port.read(&mut chunk).map_err(|e| {
                    error!("failed to read controller response: {}", e);
                    Error::Unknown
                })?;
                response.extend_from_slice(&chunk[..n]);
            }

            debug!(
                "Received response from controller: {:?}",
                str::from_utf8(&*response).expect("controller response isn't valid utf-8")
            );
            Ok(response) as Result<Vec<u8>, Error>
        })
        .await
        .map_err(|e| {
            error!(
                "failed to spawn blocking task to send controller command: {}",
                e
            );
            Error::Unknown
        })??;

        let length = result.len();
        if length > response.len() {
            return Err(Error::ResponseTooLong);
        }

        response[..length].copy_from_slice(&result[..length]);
        Ok(length)
    }
}
