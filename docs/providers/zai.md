# Z.AI Provider

Forge CLI supports Z.AI (international version) as a model provider through OpenAI-compatible API.

## Overview

Z.AI provides GLM models (glm-5.1, glm-4.7, glm-4.5, glm-4.5-air) through an OpenAI-compatible API endpoint.

## Supported Models

- `glm-5.1` - Latest flagship model (best performance)
- `glm-4.7` - High-performance model
- `glm-4.5` - Standard model
- `glm-4.5-air` - Lightweight/faster model (cost-effective)

## API Key

1. Visit [Z.AI Open Platform](https://z.ai)
2. Register or login
3. Create API Key in API Keys management page
4. Set environment variable: `export ZAI_API_KEY=your_api_key`

## Usage

### Basic Usage

```rust
use forge_provider::OpenAIProvider;

let provider = OpenAIProvider::with_base_url(
    "glm-5.1",
    std::env::var("ZAI_API_KEY").unwrap(),
    "https://api.z.ai/api/paas/v4/chat/completions"
);

let response = provider.chat(&messages).await?;
```

### Configuration

The Z.AI endpoint is: `https://api.z.ai/api/paas/v4/chat/completions`

Required headers are automatically added:
- `Authorization: Bearer YOUR_API_KEY`
- `Content-Type: application/json`

## Testing

Run integration test with real Z.AI API:

```bash
# Set your API key
export ZAI_API_KEY=your_api_key

# Run integration test
cargo test --package forge-provider test_zai_real_api -- --ignored
```

## References

- [Z.AI API Documentation](https://docs.z.ai/api-reference/introduction)
- [Z.AI HTTP API Guide](https://docs.z.ai/guides/develop/http/introduction)
