#![doc = include_str!("../README.md")]
#![no_std]
#![no_main]
#![allow(async_fn_in_trait)]

// Logging and panicking behavior can be customized by implementing the `defmt::Logger`
// and `core::panic::PanicInfo` traits, respectively.
use {defmt_rtt as _, panic_probe as _};

use embassy_executor::Spawner;
use embassy_net::StackResources;
use embassy_rp::{
    bind_interrupts,
    clocks::RoscRng,
    gpio::{Level, Output},
    peripherals::{DMA_CH0, PIO0},
    pio::{InterruptHandler as PioInterruptHandler, Pio},
};

use cyw43::JoinAuth;
use cyw43::JoinOptions;
use cyw43::NetDriver;
use cyw43_firmware::{CYW43_43439A0, CYW43_43439A0_CLM};
use cyw43_pio::{DEFAULT_CLOCK_DIVIDER, PioSpi};

use core::str::FromStr;
use heapless::String;
use static_cell::StaticCell;

use nanooctopus::*;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => PioInterruptHandler<PIO0>;
});

const NETWORK_STACK_SOCKETS: usize = 20;

static NETWORK_RESOURCES: StaticCell<StackResources<NETWORK_STACK_SOCKETS>> = StaticCell::new();
static CY43_STATE: StaticCell<cyw43::State> = StaticCell::new();

const SOCKETS: usize = 5; // Number of simultaneous sockets the server can accept and handle; adjust as needed
const RX_SIZE: usize = 256; // Size of the receive buffer for each socket; adjust as needed
const TX_SIZE: usize = 256; // Size of the transmit buffer for each socket; adjust as needed
const WORKER_MEMORY: usize = 4096; // Size of the worker memory buffer for parsing HTTP headers; adjust as needed
const HTTP_SERVER_WORKERS: usize = 2; // Number of worker tasks for handling HTTP requests; adjust as needed
const HTTP_SERVER_PORT: u16 = 8080; // Port for the HTTP server to listen on; adjust as needed

static SOCKET_BUFFER: StaticCell<[u8; SOCKETS * (RX_SIZE + TX_SIZE)]> = StaticCell::new(); // Buffer for the socket pool; adjust size as needed
static SOCKET_POOL_STATE: StaticCell<server::socket_pool::TcpSocketPoolState<'static, SOCKETS>> = StaticCell::new(); // State for the socket pool
static SERVER_INSTANCE: StaticCell<server::HttpServer<server::socket_pool::TcpSocketPool<'static, SOCKETS>>> =
    StaticCell::new(); // HTTP server instance

type PioSpi0 = PioSpi<'static, PIO0, 0, DMA_CH0>;
type Cy43Runner = cyw43::Runner<'static, Output<'static>, PioSpi<'static, PIO0, 0, DMA_CH0>>;
type NetStackRunner = embassy_net::Runner<'static, NetDriver<'static>>;

/// Just returns a mask string. Nothing fancy, but it prevents the password from being accidentally logged in plaintext.
const fn mask_password(_: &str) -> &'static str {
    "******"
}

#[embassy_executor::main]
async fn main(spawner: Spawner) -> ! {
    /**************************************************************************************************/
    /*                                Initialize the network stack                                    */
    /**************************************************************************************************/

    // These environment variables are forwarded from the build script, which reads them from the .env file.
    // This allows us to keep the WiFi credentials out of the source code and instead manage them through environment variables.
    // Create the .env file in the project root with the following content:
    // WIFI_SSID=your_wifi_ssid
    // WIFI_PASSWORD=your_wifi_password
    let ssid: String<32> = heapless::String::from_str(option_env!("WIFI_SSID").unwrap_or("None")).unwrap();
    let password = heapless::String::<64>::from_str(option_env!("WIFI_PASSWORD").unwrap_or("")).unwrap();

    // Initialize peripherals
    let p: embassy_rp::Peripherals = embassy_rp::init(Default::default());

    let pwr: Output<'_> = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let dma = p.DMA_CH0;

    let spi: PioSpi0 = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        DEFAULT_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        dma,
    );

    // Firmware binary included in the cyw43_firmware crate;
    let fw = CYW43_43439A0;

    defmt::info!("Creating WiFi driver...");
    let cyw43_state = CY43_STATE.init(cyw43::State::new());
    let (wifi_network_driver, mut control, cyw43_runner) = cyw43::new(cyw43_state, pwr, spi, fw).await;
    defmt::info!("WiFi driver created.");

    // Spawn the CYW43 runner task. Spawning this task here guarantees the WiFi driver operates correctly.
    spawner.spawn(wifi_runner_task(cyw43_runner)).unwrap();

    // Initialize the WiFi hardware with CLM data
    defmt::debug!("Initializing WiFi driver...");
    let clm = CYW43_43439A0_CLM; // CLM binary included in the cyw43_firmware crate;
    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::Performance)
        .await;
    defmt::info!("WiFi driver initialized.");

    defmt::info!("Attempting to join SSID: {}", ssid.as_str());
    defmt::info!("Attempting to join with password: {}", mask_password(password.as_str()));

    let mut join_options = JoinOptions::default();
    join_options.auth = JoinAuth::Wpa2Wpa3;
    join_options.passphrase = password.as_str().as_bytes();

    while let Err(e) = control.join(ssid.as_str(), join_options.clone()).await {
        defmt::error!(
            "Failed to join WiFi network: {:?}. Retrying...",
            defmt::Debug2Format(&e)
        );
    }
    defmt::info!("Successfully joined WiFi network");

    defmt::info!("Configuring network stack...");
    let stack_resources = NETWORK_RESOURCES.init(StackResources::new());

    defmt::info!("configuring network stack with DHCP");
    let mut rng = RoscRng;
    let seed = rng.next_u64();
    let (net_stack, runner) = embassy_net::new(
        wifi_network_driver,
        embassy_net::Config::dhcpv4(Default::default()),
        stack_resources,
        seed,
    );
    spawner.spawn(wifi_network_runner(runner)).unwrap();

    net_stack.wait_link_up().await;
    defmt::info!("Network link is up.");
    net_stack.wait_config_up().await;
    defmt::info!("Network configuration is up.");

    let config = net_stack.config_v4().unwrap_or_else(|| {
        defmt::panic!("Failed to get network configuration.");
    });

    defmt::info!("IPv4 address: {}", config.address);
    defmt::info!("IPv4 gateway: {}", config.gateway);
    defmt::info!("IPv4 DNS servers: {:?}", config.dns_servers);

    /**************************************************************************************************/
    /*                                    Initialize the server                                       */
    /**************************************************************************************************/

    // Create a local endpoint for the server to listen on (e.g., port 8080)
    // For embassy-net, the IP address is not used as it is determined by the network stack
    // configuration, but we still need to provide a valid SocketAddr.
    // We can use the wildcard address (0.0.0.0) to listen on all available interfaces or
    //just use the assigned IP for consistency.
    let local_endpoint = server::SocketEndpoint::new(core::net::IpAddr::V4(config.address.address()), HTTP_SERVER_PORT);

    // Initialize the buffer for the socket pool. It needs to be large enough to hold the RX and TX buffers for all sockets.
    let buffer = SOCKET_BUFFER.init_with(|| [0u8; SOCKETS * (RX_SIZE + TX_SIZE)]);

    // Initialize the socket pool state with the network stack and local endpoint. This will be used by the TCP socket pool
    // server to manage incoming connections.
    let socket_pool_state = SOCKET_POOL_STATE.init_with(|| {
        server::socket_pool::TcpSocketPoolState::new::<RX_SIZE, TX_SIZE>(net_stack, buffer, local_endpoint)
    });

    // Create the TCP socket pool, which will manage incoming TCP connections for the server. The socket pool will use the
    // provided state to accept and manage sockets.
    let (socket_pool, runner) = server::socket_pool::TcpSocketPool::new(core::pin::Pin::new(socket_pool_state));

    // Spawn the socket pool runner task. This task will continuously run in the background, accepting incoming TCP connections
    // and managing the socket pool.
    spawner.spawn(socket_pool_runner(runner)).unwrap();

    // Create the HTTP server with the socket pool and default timeouts. The server will use the socket pool to accept incoming
    // connections and will manage the HTTP request handling.
    let server = SERVER_INSTANCE.init_with(|| server::HttpServer::new(socket_pool, server::ServerTimeouts::default()));

    // Spawn worker tasks for the HTTP server. Each worker will handle incoming HTTP requests concurrently.
    // The number of workers can be adjusted based on the expected load and resource constraints of the device.
    for worker_id in 0..HTTP_SERVER_WORKERS {
        spawner.spawn(http_server_task(server, worker_id)).unwrap();
    }

    defmt::info!("\n\nHTTP server is running and ready to accept requests.");
    defmt::info!("Visit http://{}:{}/", config.address.address(), HTTP_SERVER_PORT);

    defmt::info!("To check the number of active connections the server can handle, run the script from project root:");
    defmt::info!(
        "./scripts/hold_open_load.py -c {} --host {} --port {}\n\n",
        SOCKETS,
        config.address.address(),
        HTTP_SERVER_PORT
    );

    loop {
        embassy_futures::yield_now().await;
    }
}

#[embassy_executor::task]
async fn wifi_runner_task(runner: Cy43Runner) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn wifi_network_runner(mut net_runner: NetStackRunner) -> ! {
    net_runner.run().await
}

#[embassy_executor::task]
async fn socket_pool_runner(runner: server::socket_pool::TcpSocketPoolRunner<'static, SOCKETS>) -> ! {
    runner.run().await
}

/// Request handler that responds to every HTTP request with "Hello, World!".
struct HelloWorldHandler;

impl http_handler::HttpHandler for HelloWorldHandler {
    async fn handle_request(
        &mut self,
        _allocator: &mut http_handler::HttpAllocator<'_>, // unused in this simple handler
        request: &http_handler::HttpRequest<'_>,
        http_socket: &mut impl http_handler::HttpSocketWrite,
        context_id: usize,
    ) -> Result<http_handler::HttpResponse, http_handler::Error> {
        defmt::debug!("HelloWorldHandler[{}]: Received request: {:?}", context_id, request);

        let mut greetings: heapless::String<128> = heapless::String::new();
        let _ = core::fmt::write(
            &mut greetings,
            format_args!("Hello, World from worker {}!\n", context_id),
        );

        // Stream the response directly to the socket: status → headers → body.
        http_handler::HttpResponseBuilder::new(http_socket)
            .with_status(http_handler::StatusCode::Ok)
            .await?
            .with_header("Content-Type", "text/plain")
            .await?
            .with_body_from_slice(greetings.as_bytes())
            .await
    }
}

#[embassy_executor::task(pool_size = HTTP_SERVER_WORKERS)]
async fn http_server_task(
    server: &'static server::HttpServer<server::socket_pool::TcpSocketPool<'static, SOCKETS>>,
    worker_id: usize,
) -> ! {
    let mut worker_memory_buf = [core::mem::MaybeUninit::<u8>::uninit(); WORKER_MEMORY];
    let worker_memory = server::HttpWorkerMemory::new(&mut worker_memory_buf);
    server.serve(worker_memory, HelloWorldHandler, worker_id).await
}
