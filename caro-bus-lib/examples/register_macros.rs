use caro_bus_lib::{service, Service};

#[service("caro.bus.register_service")]
struct RegisterService {}

#[tokio::main]
async fn main() {
    let mut reg_service = RegisterService {};
    reg_service.init();
    reg_service.print();
}
