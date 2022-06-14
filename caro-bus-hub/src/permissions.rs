use log::*;

pub fn service_name_allowed(socket_addr: &String, service_name: &String) -> bool {
    trace!(
        "Checking if binary {} allowed to register service {}",
        socket_addr,
        service_name
    );

    true
}

pub fn connection_allowed(client_service: &String, target_service: &String) -> bool {
    trace!(
        "Checking if connection from {} to {} allowed",
        client_service,
        target_service
    );

    true
}
