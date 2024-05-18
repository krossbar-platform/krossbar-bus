use std::{
    fs::{read_link, File},
    io::Read,
    path::PathBuf,
};

use glob::{self, Pattern};
use json::JsonValue;
use log::*;
use tokio::net::unix::UCred;

use krossbar_bus_common::{CONNECT_SERVICE_NAME, DEFAULT_SERVICE_FILES_DIR, MONITOR_SERVICE_NAME};
use krossbar_common_rpc::{Error, Result};

use crate::service_names::NamePattern;

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
/// *allowed_connections* supports service name patterns. See [krossbar_bus_common::service_names] for details
pub struct Permissions {
    service_files_dirs: Vec<PathBuf>,
}

impl Permissions {
    /// Creates new permissions handle.
    /// Caller must ensure the directory exists
    pub fn new(additional_service_dirs: &Vec<PathBuf>) -> Self {
        let mut service_files_dirs: Vec<PathBuf> = Vec::new();
        service_files_dirs.push(DEFAULT_SERVICE_FILES_DIR.into());

        for dir in additional_service_dirs {
            service_files_dirs.push(dir.into());
        }

        Self { service_files_dirs }
    }

    /// Check if a process alowed to register with a given **service_name**
    pub fn check_service_name_allowed(&self, user_credentials: UCred, service_name: &str) -> bool {
        trace!("Incoming service name check for `{}`", service_name);

        let pid = match user_credentials.pid() {
            Some(pid) => pid,
            _ => {
                warn!("Failed to get peer `{}` pid", service_name);
                return false;
            }
        };
        trace!("Peer pid: {}", pid);

        let process_exe_path = format!("/proc/{}/exe", pid);

        let service_exec = match read_link(&process_exe_path) {
            Ok(buf) => buf.to_str().unwrap().to_string(),
            _ => {
                warn!("Failed to get exec path for PID {}", pid);
                return false;
            }
        };
        trace!("Peer binary: {}", service_exec);

        debug!(
            "Checking if binary `{}` allowed to register service {}",
            service_exec, service_name
        );

        let exec_pattern = match self.read_allowed_execs(service_name) {
            Ok(pattern) => pattern,
            _ => {
                return false;
            }
        };

        trace!("Matching {:?} over {:?}", service_exec, exec_pattern);

        if exec_pattern.matches(&service_exec) {
            true
        } else {
            warn!(
                "Binary `{}` is not allowed to register `{}` service",
                service_exec, service_name
            );

            false
        }
    }

    /// Read allowed executables for a given service from a service file
    fn read_allowed_execs(&self, service_name: &str) -> Result<Pattern> {
        let json = self.parse_service_file_json(service_name)?;

        if !json.has_key(ALLOWED_EXECS_KEY) {
            warn!(
                "Failed to find `{}` entry in a service file",
                ALLOWED_EXECS_KEY
            );
            return Err(Error::NotAllowed);
        }

        let allowed_execs = match json[ALLOWED_EXECS_KEY].as_str() {
            Some(str) => str,
            _ => {
                warn!(
                    "Invalid `{}` entry in a service file. Expected string, got `{}`",
                    ALLOWED_EXECS_KEY, json[ALLOWED_EXECS_KEY]
                );
                return Err(Error::NotAllowed);
            }
        };

        match Pattern::new(allowed_execs) {
            Ok(glob) => Ok(glob),
            Err(glob_err) => {
                warn!( "Invalid `{}` entry in a service file. Failed to parse the entry as a GLOB patter: {}",
                ALLOWED_EXECS_KEY, glob_err.to_string());
                return Err(Error::NotAllowed);
            }
        }
    }

    /// Check if a **client_service** is allowed to connect to a **target_service**
    pub fn check_connection_allowed(&self, client_service: &str, target_service: &str) -> bool {
        trace!(
            "Checking if connection from {} to {} allowed",
            client_service,
            target_service
        );

        if self.is_previleged_service(client_service) {
            debug!("Connection from a previleged service `{}`", client_service);
            return true;
        }

        match self.read_allowed_connections(target_service) {
            Ok(patterns) => {
                for pattern in patterns {
                    trace!("Matching {:?} over {:?}", client_service, pattern);

                    match pattern.matches(client_service) {
                        Ok(matches) if matches => {
                            return true;
                        }
                        Err(err) => {
                            warn!(
                                "Failed to match `{}` service name against name pattern: {}",
                                client_service,
                                err.to_string()
                            );
                            return false;
                        }
                        Ok(_) => {}
                    };
                }
            }
            _ => {
                return false;
            }
        }

        warn!(
            "Service `{}` is not allowed to connect to `{}`",
            client_service, target_service
        );

        false
    }

    /// Read allowed incoming connections for a given service from a service file
    fn read_allowed_connections(&self, service_name: &str) -> Result<Vec<NamePattern>> {
        let mut result = vec![];

        let json = self.parse_service_file_json(service_name)?;

        if !json.has_key(INCOMING_CONNS_KEY) {
            warn!(
                "Failed to find `{}` entry in a service file",
                INCOMING_CONNS_KEY
            );
            return Err(Error::NotAllowed);
        }

        if !json[INCOMING_CONNS_KEY].is_array() {
            warn!(
                "Invalid `{}` entry in a service file. Expected array, got `{}`",
                INCOMING_CONNS_KEY, json[INCOMING_CONNS_KEY]
            );
            return Err(Error::NotAllowed);
        }

        for connection_entry in json[INCOMING_CONNS_KEY].members() {
            let connection_string = match connection_entry.as_str() {
                Some(str) => str,
                _ => {
                    warn!(
                        "Invalid connection entry in a service file. Expected string, got `{}`",
                        connection_entry
                    );
                    return Err(Error::NotAllowed);
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
                    return Err(Error::NotAllowed);
                }
            }
        }

        Ok(result)
    }

    fn parse_service_file_json(&self, service_name: &str) -> Result<JsonValue> {
        let maybe_service_file = self.service_files_dirs.iter().find_map(|dir| {
            let service_file_name = dir.join(format!("{}.service", service_name));

            File::open(service_file_name.as_path())
                .ok()
                .map(|f| (f, service_file_name))
        });

        if maybe_service_file.is_none() {
            warn!("Failed to find service file for a {service_name} service");

            return Err(Error::ServiceNotFound);
        }

        let mut service_file_string = String::new();
        let (mut file, path) = maybe_service_file.unwrap();

        if let Err(err) = file.read_to_string(&mut service_file_string) {
            warn!(
                "Failed to read {service_name} service file at `{path:?}`: {}",
                err.to_string()
            );
            return Err(Error::NotAllowed);
        }

        match json::parse(&service_file_string) {
            Err(parse_error) => {
                warn!(
                    "Failed to parse service file at `{path:?}`: {}",
                    parse_error.to_string()
                );
                return Err(Error::NotAllowed);
            }
            Ok(value) => Ok(value),
        }
    }

    /// Returns if service allows to connect to any counterparty
    fn is_previleged_service(&self, service_name: &str) -> bool {
        service_name == MONITOR_SERVICE_NAME || service_name == CONNECT_SERVICE_NAME
    }
}
