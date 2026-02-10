# plug_pricing

Sample plugin implementation for CoverZen MCP plugins.

## Description

This project demonstrates how to build a plugin for the CoverZen MCP ecosystem using Rust. 
It serves as a reference implementation and demonstrates the use of the macros 
provided in `mcp-plugin-api`. 

It specifically demonstrates & discusses how to implement async MCP function calls within a plugin.


## Context

A rust MCP plugin within the *ConverZen* architecture has a C API interface that knows nothing
of async and await. So the rust function being called as a tool handler inside the plugin is called
synchronously but from within an async tokio runtime (that of the MCP server).
From the perspective of the MCP server its worker task  calls a synchronous function that
blocks its executing until it returns.

As a result in most cases using an asynchronous runtime inside the plugin makes no sense.
You are better of using synchronous tools. That is also true for this plugin implementation. 
Nevertheless we found that it makes sense to show how it should be done from our point of view.


In the following we discuss when it makes sense to go async inside the plugin and how it is done.

This architectural challenge is a classic "bridge" problem: how to move data safely and efficiently between an asynchronous runtime and a synchronous C-style interface without crashing the host process.

### 1. When does this architecture make sense?

Using a dedicated asynchronous runtime behind a synchronous interface is **only** worth the complexity if your plugin's internal tasks meet these criteria:

* **Internal Parallelism:** You need to perform multiple I/O operations simultaneously (e.g., fetching a DB record and an API response at the same time) to reduce total latency.
* **Async-Only Ecosystem:** You are using a library (like certain specialized MCP toolkits or Cloud drivers) that does not provide a synchronous/blocking version of its API.
* **Task Management:** You need complex features like timeouts, retries, or "select" logic across multiple streams that are cumbersome to write in pure synchronous code.

**If you are only making one single database call, it is objectively better to use a synchronous library.**

---

### 2. The Primary Danger: The "Nesting Panic"

The biggest risk in a plugin environment is the **Recursive Runtime Panic**.

Your host (the MCP server) is already running a Tokio runtime - it calls your plugin function. If your plugin tries to call `tokio::runtime::Runtime::block_on()`, the program will crash instantly.

> **Error:** *"Cannot start a runtime from within a runtime. This happens because block_on attempts to take over the thread's event loop, which is already owned by the host."*

---

### 3. The Solution: The "Dedicated Bridge" Pattern

To avoid the panic and keep performance high, you must isolate your async work on a separate OS thread and use a lightweight, non-Tokio bridge to wait for the result.

#### Step A: The Background "Engine"

In the plugins init function, you initialize a static runtime on its own thread. This thread "owns" the event loop, 
so it never conflicts with the host. 

```rust
fn ensure_runtime() -> &'static mpsc::UnboundedSender<Command> {
    TX.get_or_init(|| {
        let (tx, mut rx) = mpsc::unbounded_channel::<Command>();

        let (init_tx, init_rx) = oneshot::channel::<InitResult>();
        // Spawn a dedicated OS thread for our async world
        std::thread::spawn(move || {
            let rt = match Runtime::new() {
                Ok(rt) => rt,
                Err(err) => {
                    let _ = init_tx.send(InitResult::Error(err.to_string()));
                    return;
                }
            };
            rt.block_on(async {
                // async initialization here
                ...
            })
        })
    }
}

fn init() -> Result<(), String> {
    let config = get_config();

    // Create the async runtime
    let _tx = ensure_runtime();

    Ok(())
}

```

#### Step B: The Sync-to-Async Bridge

Inside your C-style sync function, you send the work to the background thread and use 
a generic `oneshot` receiver to block the host thread until the work is done.

```rust
// ============================================================================
// Tool Handlers
// ============================================================================

/// Handler for get_product_price tool
fn handle_get_product_price_sync(args: &Value) -> Result<Value, String> {
    let tx = ensure_runtime();
    let (resp_tx, resp_rx) = oneshot::channel();

    // 1. Offload work to the dedicated runtime
    tx.send(Command::GetProductPrice(McpRequest {
        payload: args.clone(),
        responder: resp_tx,
    })).ok();

    // 2. BLOCK the host thread using a light-weight executor
    // This does NOT try to start a new runtime, so it won't panic.
    futures::executor::block_on(resp_rx).map_err(|err| err.to_string())?
}

```

---

### 4. Summary of Architecture Logic

1. **Isolation:** By using `std::thread::spawn`, you create a private playground for Tokio. The host can be running its own Tokio, or none at all; your plugin won't care.
2. **Concurrency:** Because the background runtime is multi-threaded, it can handle multiple calls from the host at once. The host threads wait at the `oneshot` receiver while your internal threads do the heavy lifting.
3. **Stability:** Using `futures::executor::block_on` is the key. It is a "dumb" waiter that simply parks the thread until a signal arrives. It is safe to use even if the thread belongs to a different Tokio runtime.

## Prerequisites

- Rust (latest stable version)
- Cargo


## Building

To build the plugin, run:

```bash
cargo build --release
```

The compiled artifact will be located in `target/release/`.

## Configuration

### 2. Setup Database (for pricing plugin)

```bash
# Set database URL
export DATABASE_URL="postgresql://user:password@localhost/products"

# Create sample table (example)
psql $DATABASE_URL << EOF
CREATE TABLE IF NOT EXISTS products (
    id SERIAL PRIMARY KEY,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    price DECIMAL(10,2) NOT NULL,
    stock INTEGER NOT NULL DEFAULT 0
);

INSERT INTO products (name, description, price, stock) VALUES
    ('Widget Pro', 'Professional grade widget', 29.99, 100),
    ('Gadget Plus', 'Enhanced gadget with features', 49.99, 50);
EOF
```

## License

MIT or Apache-2.0
