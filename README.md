# plug_pricing

Sample plugin implementation for CoverZen MCP plugins.

## Description

This project demonstrates how to build a plugin for the CoverZen MCP ecosystem using Rust. 
It serves as a reference implementation and demonstrates the use of the macros 
provided in `mcp-plugin-api`. 

It specifically demonstrates how to implement async MCP function calls within a plugin.

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
