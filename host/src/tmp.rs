use crate::controller::{Controller, Destination};
use common::ControllerError;
use log::*;
use std::sync::Arc;
use thiserror::Error;

/// An error returned from the TMP.
#[derive(Debug, Error)]
pub enum TmpError {
    /// An error returned from the vacuum system controller.
    ///
    /// Note that this isn't an error returned from the ADC, but may be
    /// encountered when we try to communicate with it (e.g. a timeout).
    #[error("controller error: {0}")]
    Controller(#[from] ControllerError),

    // Telegram errors
    #[error("parameter does not exist: {0}")]
    InvalidParameter(String),
    #[error("logic error")]
    Logic,
    #[error("value outside permitted range")]
    ValueOutsideRange,

    // Custom errors
    #[error("invalid checksum")]
    InvalidChecksum,
    #[error("invalid response")]
    InvalidResponse,
    #[error("non-ascii character: {0}")]
    NonAsciiCharacter(char),
}

/// Describes the action of a telegram (command or response).
enum Action {
    /// The telegram contains a description of a parameter (its value).
    ///
    /// When used in a command, this sets the value of a parameter. This is used
    /// in all responses to state the value of the parameter that was read/set.
    Describe,

    /// The telegram contains a request to read the value of a parameter.
    ///
    /// This can only be used in commands.
    Read,
}

/// Interacts with the Pfeiffer TC 600 TMP controller.
///
/// Only some parameters are supported. See the "Pumping Operations With DCU"
/// document for a full list of all parameters supported by the TC 600.
pub struct Tmp {
    address: String,
    controller: tokio::sync::Mutex<Arc<Controller>>,
}

impl Tmp {
    pub fn new(address: &str, controller: Arc<Controller>) -> Self {
        Self {
            address: address.to_string(),
            controller: tokio::sync::Mutex::new(controller),
        }
    }

    pub async fn get_operating_hours(&self) -> Result<i32, TmpError> {
        let controller = self.controller.lock().await;
        let response = self
            .send_command(&controller, Action::Read, "311", "=?")
            .await?;
        Ok(response
            .parse::<i32>()
            .map_err(|_| TmpError::InvalidResponse)?)
    }

    async fn send_command(
        &self,
        controller: &Controller,
        action: Action,
        parameter_number: &str,
        data: &str,
    ) -> Result<String, TmpError> {
        // Construct the command to send.
        let mut command = format!(
            "{}{}{}{:02}{}",
            self.address,
            match action {
                Action::Describe => "10",
                Action::Read => "00",
            },
            parameter_number,
            data.len(),
            data,
        );
        command.push_str(&format!("{:03}", Self::calculate_checksum(&command)?));

        // Send the command and wait for a response.
        let response = controller.send_command(Destination::TMP, &command).await?;

        // The address must equal our address.
        let response_address = &response[..3];
        if response_address != self.address {
            error!(
                "expected response address {}, got {}",
                self.address, response_address
            );
            return Err(TmpError::InvalidResponse);
        }

        // The action must be 10 (describe).
        let response_action = &response[3..5];
        if response_action != "10" {
            error!("expected response action 10, got {}", response_action);
            return Err(TmpError::InvalidResponse);
        }

        // The parameter number must match.
        let response_parameter_number = &response[5..8];
        if response_parameter_number != parameter_number {
            error!(
                "expected response parameter {}, got {}",
                parameter_number, response_parameter_number
            );
            return Err(TmpError::InvalidResponse);
        }

        // The data length must be a valid number.
        let data_length = &response[8..10];
        let data_length = data_length.parse::<usize>().map_err(|_| {
            error!("couldn't parse response data length {}", data_length);
            TmpError::InvalidResponse
        })?;

        // The response length must agree with `data_length`.
        let expected_response_length = 3 + 2 + 3 + 2 + data_length + 3;
        if response.len() != expected_response_length {
            error!(
                "expected response length {}, got {}",
                expected_response_length,
                response.len()
            );
            return Err(TmpError::InvalidResponse);
        }

        // Determine if the response is an error.
        let response_data = &response[10..10 + data_length];
        match response_data {
            "_LOGIC" => {
                error!("logic error");
                return Err(TmpError::Logic);
            }
            "_RANGE" => {
                error!("parameter value was outside the permitted range");
                return Err(TmpError::ValueOutsideRange);
            }
            "NO_DEF" => {
                error!("parameter does not exist {}", response_parameter_number);
                return Err(TmpError::InvalidParameter(String::from(
                    response_parameter_number,
                )));
            }
            // Without this, rustc thinks the code below is unreachable.
            _ => {}
        }

        // Validate the response checksum.
        let response_checksum = &response[10 + data_length..];
        let response_checksum = response_checksum.parse::<i32>().map_err(|_| {
            error!("couldn't parse response checksum {}", response_checksum);
            TmpError::InvalidResponse
        })?;
        if response_checksum != Self::calculate_checksum(&response[..10 + data_length])? {
            error!("invalid response checksum");
            return Err(TmpError::InvalidChecksum);
        }

        Ok(String::from(response_data))
    }

    fn calculate_checksum(s: &str) -> Result<i32, TmpError> {
        let mut checksum: i32 = 0;
        for c in s.chars() {
            if !c.is_ascii() {
                error!("non-ascii character: {}", c);
                return Err(TmpError::NonAsciiCharacter(c));
            }
            checksum += c as i32;
        }
        Ok(checksum % 256)
    }
}
