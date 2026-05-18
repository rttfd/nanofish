# rasberry_pico_w

An embedded **nanooctopus** example for the **Raspberry Pi Pico W** using
**Embassy**, **CYW43 Wi-Fi**, and the RP2040.
It connects to a Wi-Fi network, acquires an IP address over DHCP, and starts a
small HTTP server on port `8080`.

Each request receives a plain-text response similar to:

```text
Hello, World from worker 0!
```

The exact worker number depends on which HTTP worker task handled the request.

______________________________________________________________________

## What this example demonstrates

| Concept                                              | Where               |
| ---------------------------------------------------- | ------------------- |
| Bringing up CYW43 Wi-Fi on Pico W                    | `main`              |
| Configuring Embassy networking with DHCP             | `main`              |
| Running a TCP socket pool                            | `TcpSocketPool`     |
| Serving HTTP with multiple worker tasks              | `http_server_task`  |
| Implementing `HttpHandler` on embedded               | `HelloWorldHandler` |
| Streaming a response with `HttpResponseBuilder`      | `handle_request`    |
| Passing Wi-Fi credentials from `.env` via `build.rs` | `build.rs` + `main` |

______________________________________________________________________

## Key components

### `HelloWorldHandler`

Implements `http_handler::HttpHandler` and replies to every request with a
short plain-text body.
The response is streamed directly to the socket in three stages:

```text
status line  →  headers  →  body
```

This keeps the response path allocation-free in the normal embedded flow.

### Socket pool and worker split

This example separates connection management from request execution:

| Resource              | Value  |
| --------------------- | ------ |
| `SOCKETS`             | `5`    |
| `HTTP_SERVER_WORKERS` | `2`    |
| `WORKER_MEMORY`       | `4096` |
| `RX_SIZE` / `TX_SIZE` | `256`  |
| HTTP port             | `8080` |

That means the firmware can keep more TCP sockets open than there are active
HTTP request workers, which is useful for browsers and clients that open
multiple connections eagerly.

### Wi-Fi configuration via `.env`

The build script reads `.env` from the example root and forwards these values
into the firmware build:

- `WIFI_SSID`
- `WIFI_PASSWORD`

At runtime, the firmware reads them through `option_env!()` and uses them to
join the access point.

### Runner configuration

The example already includes [.cargo/config.toml](.cargo/config.toml), which sets:

- the default target to `thumbv6m-none-eabi`
- the default runner to `probe-rs run --chip RP2040 --protocol swd`

Because of that, plain `cargo build` and `cargo run` work from this directory
without needing to repeat the target triple each time.

______________________________________________________________________

## Prerequisites

You need:

- a Raspberry Pi Pico W
- a SWD debug probe connected to the board
- Rust installed through `rustup`
- the `thumbv6m-none-eabi` target installed
- `probe-rs` installed and available in `PATH`

Install the Rust target:

```sh
rustup target add thumbv6m-none-eabi
```

Install `probe-rs` if needed:

```sh
cargo install probe-rs-tools
```

______________________________________________________________________

## Wi-Fi setup

All `cargo` commands for this example should be run from the
`demos/rasberry_pico_w/` directory.

```sh
cd demos/rasberry_pico_w
```

Create a local `.env` file in that directory:

```env
WIFI_SSID=your_wifi_ssid
WIFI_PASSWORD=your_wifi_password
```

The `.env` file is read by `build.rs` during compilation. If you change Wi-Fi
credentials, Cargo will rebuild automatically.

______________________________________________________________________

## Building and running

### Build only

```sh
cargo build
```

This builds the firmware for `thumbv6m-none-eabi`.

### Build in release mode

```sh
cargo build --release
```

### Run on hardware

```sh
cargo run
```

Because the example configures `probe-rs` as the runner, this command will:

1. build the firmware
2. flash it to the RP2040
3. start the program
4. stream RTT/`defmt` logs in the terminal

You should see log lines for Wi-Fi join, DHCP configuration, and finally a line
similar to:

```text
HTTP server is running and ready to accept requests.
Visit http://<device-ip>:8080/
```

### Run a release build on hardware

```sh
cargo run --release
```

### Explicit target build

If you prefer to be explicit or want to override local config expectations:

```sh
cargo build --target thumbv6m-none-eabi
```

______________________________________________________________________

## Verifying the server

Once the device logs its DHCP address, send a request from another machine on
the same network, or from the host if it can reach the board:

```sh
curl http://<device-ip>:8080/
```

Expected response:

```text
Hello, World from worker 0!
```

The worker number may be different.

For verbose HTTP output:

```sh
curl -v http://<device-ip>:8080/
```

The firmware also prints a suggested load-check command:

```sh
./scripts/hold_open_load.py -c 2 --host <device-ip> --port 8080
```

Run that command from the repository root to exercise the socket-pool behavior.

______________________________________________________________________

## Logging

The example enables `defmt`, `defmt-rtt`, and `panic-probe`.
Default embedded log level is configured in [.cargo/config.toml](.cargo/config.toml) as:

```toml
DEFMT_LOG = "info"
```

At that level you will typically see:

- Wi-Fi join progress
- network link and DHCP status
- assigned IPv4 address
- server startup messages

If you need more detailed output, adjust `DEFMT_LOG` in the example's local
Cargo config.

______________________________________________________________________

## Troubleshooting

### `WIFI_SSID` or `WIFI_PASSWORD` seems empty

Make sure the `.env` file exists in `demos/rasberry_pico_w/` and rebuild.

### `cargo run` cannot talk to the board

Check that:

- the debug probe is connected over SWD
- the Pico W is powered
- `probe-rs` is installed
- the chip is detected as `RP2040`

### The board boots but does not get an IP address

Check the Wi-Fi credentials, confirm the access point supports WPA2/WPA3 as
used by the example, and verify the board has signal.
