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
            _ => {
                warn!("Failed to get peer `{}` pid", service_name);
                return Err(BusError::NotAllowed);
            }
        };
        trace!("Peer pid: {}", pid);

        let process_exe_path = format!("/proc/{}/exe", pid);

        let service_exec = match read_link(&process_exe_path) {
            Ok(buf) => buf.to_str().unwrap().to_string(),
            _ => {
                warn!("Failed to get exec path for PID {}", pid);
                return Err(BusError::NotAllowed);
            }
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

            Err(BusError::NotAllowed)
        }
    }

    fn read_allowed_execs(&self, service_name: &String) -> Result<Pattern, BusError> {
        let json = self.parse_service_file_json(service_name)?;

        if !json.has_key(ALLOWED_EXECS_KEY) {
            warn!(
                "Failed to find `{}` entry in a service file",
                ALLOWED_EXECS_KEY
            );
            return Err(BusError::NotAllowed);
        }

        let allowed_execs = match json[ALLOWED_EXECS_KEY].as_str() {
            Some(str) => str,
            _ => {
                warn!(
                    "Invalid `{}` entry in a service file. Expected string, got `{}`",
                    ALLOWED_EXECS_KEY, json[ALLOWED_EXECS_KEY]
                );
                return Err(BusError::NotAllowed);
            }
        };

        match Pattern::new(allowed_execs) {
            Ok(glob) => Ok(glob),
            Err(glob_err) => {
                warn!( "Invalid `{}` entry in a service file. Failed to parse the entry as a GLOB patter: {}",
                ALLOWED_EXECS_KEY, glob_err.to_string());
                return Err(BusError::NotAllowed);
            }
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
                    warn!(
                        "Failed to match `{}` service name against name pattern: {}",
                        client_service,
                        err.to_string()
                    );
                    return Err(BusError::NotAllowed);
                }
                Ok(_) => {}
            };
        }

        warn!(
            "Service `{}` is not allowed to connect to `{}`",
            client_service, target_service
        );

        Err(BusError::NotAllowed)
    }

    fn read_allowed_connections(
        &self,
        service_name: &String,
    ) -> Result<Vec<NamePattern>, BusError> {
        let mut result = vec![];

        let json = self.parse_service_file_json(service_name)?;

        if !json.has_key(INCOMING_CONNS_KEY) {
            warn!(
                "Failed to find `{}` entry in a service file",
                INCOMING_CONNS_KEY
            );
            return Err(BusError::NotAllowed);
        }

        if !json[INCOMING_CONNS_KEY].is_array() {
            warn!(
                "Invalid `{}` entry in a service file. Expected array, got `{}`",
                INCOMING_CONNS_KEY, json[INCOMING_CONNS_KEY]
            );
            return Err(BusError::NotAllowed);
        }

        for connection_entry in json[INCOMING_CONNS_KEY].members() {
            let connection_string = match connection_entry.as_str() {
                Some(str) => str,
                _ => {
                    warn!(
                        "Invalid connection entry in a service file. Expected string, got `{}`",
                        connection_entry
                    );
                    return Err(BusError::NotAllowed);
                }
            };

            match NamePattern::from_string(connection_string) {
                Ok(pattern) => result.push(pattern),
                Err(err) => {
                    warn!(
                        "Failed to parse allowed connection entry `{}`: {}",
                        connection_string,
                        err.to_string()
                    );
                    return Err(BusError::NotAllowed);
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
                warn!(
                    "Failed to open service file at: {}",
                    service_file_name.as_os_str().to_str().unwrap()
                );
                return Err(BusError::NotAllowed);
            }
        };

        let mut service_file_string = String::new();
        if let Err(err) = service_file.read_to_string(&mut service_file_string) {
            warn!(
                "Failed to read service file at `{}`: {}",
                service_file_name.as_os_str().to_str().unwrap(),
                err.to_string()
            );
            return Err(BusError::NotAllowed);
        }

        match json::parse(&service_file_string) {
            Err(parse_error) => {
                warn!(
                    "Failed to parse service file at `{}`: {}",
                    service_file_name.as_os_str().to_str().unwrap(),
                    parse_error.to_string()
                );
                return Err(BusError::NotAllowed);
            }
            Ok(value) => Ok(value),
        }
    }
}
