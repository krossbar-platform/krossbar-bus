use std::{
    fs::{read_link, File},
    io::Read,
    path::PathBuf,
};

use glob::{self, Pattern};
use json::JsonValue;
use log::*;
use tokio::net::unix::UCred;

use caro_bus_common::{errors::Error as BusError, service_names::NamePattern};

const ALLOWED_EXECS_KEY: &str = "exec";
const INCOMING_CONNS_KEY: &str = "incoming_connections";

/// Permissions reader.
/// Each service file must be names as {service_name}.service,
/// And have following format:
/// ```json
/// {
///     "exec": "/usr/bin/service"
///     "incoming_connections": [
///         "com.service.name"
///     ]
/// }
/// ```
/// *allowed_exec_paths* supports GLOB patterns.
/// *allowed_connections* supports service name patterns. See [caro_bus_common::service_names] for details
pub struct Permissions {
    service_files_dir: PathBuf,
}

impl Permissions {
    /// Creates new permissions handle.
    /// Caller must ensure the directory exists
    pub fn new(service_files_dir: &String) -> Self {
        Self {
            service_files_dir: PathBuf::from(service_files_dir),
        }
    }

    pub fn check_service_name_allowed(
        &self,
        user_credentials: UCred,
        service_name: &String,
    ) -> Result<(), BusError> {
        trace!("Incoming service name check for `{}`", service_name);

        let pid = match user_credentials.pid() {
            Some(pid) => pid,
            _ => return Err(BusError::NotAllowed("Failed to get service pid".into())),
        };
        trace!("Peer pid: {}", pid);

        let process_exe_path = format!("/proc/{}/exe", pid);

        let service_exec = match read_link(&process_exe_path) {
            Ok(buf) => buf.to_str().unwrap().to_string(),
            _ => return Err(BusError::NotAllowed("Failed to get peer exec path".into())),
        };
        trace!("Peer binary: {}", service_exec);

        debug!(
            "Checking if binary `{:?}` allowed to register service {}",
            service_exec, service_name
        );

        let exec_pattern = self.read_allowed_execs(service_name)?;

        trace!("Matching {:?} over {:?}", service_exec, exec_pattern);

        if exec_pattern.matches(&service_exec) {
            Ok(())
        } else {
            warn!(
                "Binary `{}` is not allowed to register `{}` service",
                service_exec, service_name
            );

            Err(BusError::NotAllowed(format!(
                "Binary `{}` is not allowed to register `{}` service",
                service_exec, service_name
            )))
        }
    }

    fn read_allowed_execs(&self, service_name: &String) -> Result<Pattern, BusError> {
        let json = self.parse_service_file_json(service_name)?;

        if !json.has_key(ALLOWED_EXECS_KEY) {
            return Err(BusError::NotAllowed(format!(
                "Failed to find `{}` entry in a service file",
                ALLOWED_EXECS_KEY
            )));
        }

        let allowed_execs = match json[ALLOWED_EXECS_KEY].as_str() {
            Some(str) => str,
            _ => {
                return Err(BusError::NotAllowed(format!(
                    "Invalid `{}` entry in a service file. Expected string, got `{}`",
                    ALLOWED_EXECS_KEY, json[ALLOWED_EXECS_KEY]
                )))
            }
        };

        match Pattern::new(allowed_execs) {
            Ok(glob) => Ok(glob),
            Err(glob_err) => return Err(BusError::NotAllowed(format!(
                "Invalid `{}` entry in a service file. Failed to parse the entry as a GLOB patter: {}",
                ALLOWED_EXECS_KEY, glob_err.to_string()
            )))
        }
    }

    pub fn check_connection_allowed(
        &self,
        client_service: &String,
        target_service: &String,
    ) -> Result<(), BusError> {
        trace!(
            "Checking if connection from {} to {} allowed",
            client_service,
            target_service
        );

        for pattern in self.read_allowed_connections(target_service)? {
            trace!("Matching {:?} over {:?}", client_service, pattern);

            match pattern.matches(client_service) {
                Ok(matches) if matches => {
                    return Ok(());
                }
                Err(err) => {
                    return Err(BusError::NotAllowed(format!(
                        "Failed to match `{}` service name against name pattern: {}",
                        client_service,
                        err.to_string()
                    )));
                }
                Ok(_) => {}
            };
        }

        warn!(
            "Service `{}` is not allowed to connect to `{}`",
            client_service, target_service
        );

        Err(BusError::NotAllowed("Connection not allowed".into()))
    }

    fn read_allowed_connections(
        &self,
        service_name: &String,
    ) -> Result<Vec<NamePattern>, BusError> {
        let mut result = vec![];

        let json = self.parse_service_file_json(service_name)?;

        if !json.has_key(INCOMING_CONNS_KEY) {
            return Err(BusError::NotAllowed(format!(
                "Failed to find `{}` entry in a service file",
                INCOMING_CONNS_KEY
            )));
        }

        if !json[INCOMING_CONNS_KEY].is_array() {
            return Err(BusError::NotAllowed(format!(
                "Invalid `{}` entry in a service file. Expected array, got `{}`",
                INCOMING_CONNS_KEY, json[INCOMING_CONNS_KEY]
            )));
        }

        for connection_entry in json[INCOMING_CONNS_KEY].members() {
            let connection_string = match connection_entry.as_str() {
                Some(str) => str,
                _ => {
                    return Err(BusError::NotAllowed(format!(
                        "Invalid connection entry in a service file. Expected string, got `{}`",
                        connection_entry
                    )))
                }
            };

            match NamePattern::from_string(connection_string) {
                Ok(pattern) => result.push(pattern),
                Err(err) => {
                    return Err(BusError::NotAllowed(format!(
                        "Failed to parse allowed connection entry `{}`: {}",
                        connection_string,
                        err.to_string()
                    )))
                }
            }
        }

        Ok(result)
    }

    fn parse_service_file_json(&self, service_name: &String) -> Result<JsonValue, BusError> {
        let service_file_name = self
            .service_files_dir
            .join(format!("{}.service", service_name));

        let mut service_file = match File::open(service_file_name.as_path()) {
            Ok(file) => file,
            _ => {
                return Err(BusError::NotAllowed(format!(
                    "Failed to open service file at: {}",
                    service_file_name.as_os_str().to_str().unwrap()
                )))
            }
        };

        let mut service_file_string = String::new();
        if let Err(err) = service_file.read_to_string(&mut service_file_string) {
            return Err(BusError::NotAllowed(format!(
                "Failed to read service file at `{}`: {}",
                service_file_name.as_os_str().to_str().unwrap(),
                err.to_string()
            )));
        }

        match json::parse(&service_file_string) {
            Err(parse_error) => {
                return Err(BusError::NotAllowed(format!(
                    "Failed to parse service file at `{}`: {}",
                    service_file_name.as_os_str().to_str().unwrap(),
                    parse_error.to_string()
                )))
            }
            Ok(value) => Ok(value),
        }
    }
}
