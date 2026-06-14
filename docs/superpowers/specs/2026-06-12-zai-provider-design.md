# Z.AI Provider Implementation Design

**Date:** 2026-06-12
**Author:** Claude & Forge User
**Status:** Approved

## Overview

Add support for Z.AI (international version) as a model provider in forge CLI. Z.AI provides GLM models (glm-5.1, glm-4.7, glm-4.5, glm-4.5-air) through an OpenAI-compatible API.

## Architecture

### Approach: Configurable Base URL

Extend `OpenAIProvider` with optional `base_url` field to support custom endpoints while maintaining backward compatibility.

### Changes

**Before:**
```rust
pub struct OpenAIProvider {
    model: String,
    api_key: String,
    client: Client,
}
```

**After:**
```rust
pub struct OpenAIProvider {
    model: String,
    api_key: String,
    client: Client,
    base_url: Option<String>,  // NEW
}
```

## Implementation Details

### 1. Struct Modification

Add `base_url` field to `OpenAIProvider`:
```rust
pub struct OpenAIProvider {
    model: String,
    api_key: String,
    client: Client,
    base_url: Option<String>,
}
```

### 2. Constructor Updates

**Default constructor (existing):**
```rust
pub fn new(model: impl Into<String>, api_key: impl Into<String>) -> Self {
    Self {
        model: model.into(),
        api_key: api_key.into(),
        client: Client::new(),
        base_url: None,  // Default: OpenAI
    }
}
```

**New constructor with custom base URL:**
```rust
pub fn with_base_url(
    model: impl Into<String>,
    api_key: impl Into<String>,
    base_url: impl Into<String>
) -> Self {
    Self {
        model: model.into(),
        api_key: api_key.into(),
        client: Client::new(),
        base_url: Some(base_url.into()),
    }
}
```

### 3. Chat Method Update

Update URL selection in `chat()` method:

```rust
async fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
    let openai_messages = Self::convert_messages(messages);

    // Use custom base_url if provided, else default to OpenAI
    let url = self.base_url.as_deref()
        .unwrap_or("https://api.openai.com/v1/chat/completions");

    let response = self
        .client
        .post(url)
        .header("Authorization", format!("Bearer {}", self.api_key))
        .header("Content-Type", "application/json")
        .json(&json!({
            "model": self.model,
            "messages": openai_messages
        }))
        .send()
        .await?;

    // ... rest of error handling unchanged
}
```

## Z.AI API Specifications

### Endpoint
- **Base URL:** `https://api.z.ai/api/paas/v4/chat/completions`
- **Method:** POST
- **Format:** OpenAI-compatible

### Authentication
- **Type:** HTTP Bearer
- **Header:** `Authorization: Bearer YOUR_API_KEY`
- **Additional Headers:**
  - `Content-Type: application/json`
  - `Accept-Language: en-US,en` (optional)

### Supported Models
- `glm-5.1` - Latest flagship model
- `glm-4.7` - High-performance model
- `glm-4.5` - Standard model
- `glm-4.5-air` - Lightweight/faster model

### Request/Response Format

Identical to OpenAI format:

```json
{
  "model": "glm-5.1",
  "messages": [
    {"role": "user", "content": "Hello"}
  ],
  "temperature": 1.0
}
```

## Usage Examples

### OpenAI (Default Behavior)
```rust
let provider = OpenAIProvider::new("gpt-4", "sk-xxx");
// Uses: https://api.openai.com/v1/chat/completions
```

### Z.AI (Custom Endpoint)
```rust
let provider = OpenAIProvider::with_base_url(
    "glm-5.1",
    "zai-api-key",
    "https://api.z.ai/api/paas/v4/chat/completions"
);
// Uses: https://api.z.ai/api/paas/v4/chat/completions
```

## Data Flow

```
User Request
    ↓
forge CLI calls provider.chat(messages)
    ↓
OpenAIProvider.chat() determines URL:
    - if base_url exists → use custom URL (Z.AI)
    - else → use default OpenAI URL
    ↓
Convert messages to OpenAI format
    ↓
Make HTTP POST request to determined URL
    ↓
Provider (OpenAI or Z.AI) processes request
    ↓
Parse response and return ChatResponse
    ↓
Forge CLI receives response
```

## Error Handling

Existing error handling remains unchanged and works for both providers:

```rust
if !response.status().is_success() {
    let error_text = response.text().await?;
    return Err(anyhow::anyhow!("API error: {}", error_text));
}
```

Generic error messages work well because Z.AI uses OpenAI-compatible error format.

## Testing Strategy

### Unit Tests

**Test 1: Default OpenAI endpoint**
```rust
#[test]
fn test_openai_default_endpoint() {
    let provider = OpenAIProvider::new("gpt-4", "test-key");
    assert_eq!(provider.model(), "gpt-4");
    assert!(provider.base_url.is_none());
}
```

**Test 2: Custom Z.AI endpoint**
```rust
#[test]
fn test_zai_custom_endpoint() {
    let provider = OpenAIProvider::with_base_url(
        "glm-5.1",
        "test-key",
        "https://api.z.ai/api/paas/v4/chat/completions"
    );
    assert_eq!(provider.model(), "glm-5.1");
    assert_eq!(
        provider.base_url,
        Some("https://api.z.ai/api/paas/v4/chat/completions".to_string())
    );
}
```

**Test 3: Message conversion**
```rust
#[test]
fn test_convert_messages() {
    let messages = vec![
        Message::system("You are helpful"),
        Message::user("Hello"),
    ];
    let converted = OpenAIProvider::convert_messages(&messages);
    assert_eq!(converted.len(), 2);
    assert_eq!(converted[0]["role"], "system");
}
```

### Integration Tests (Optional)

```rust
#[tokio::test]
#[ignore]
async fn test_zai_real_api() {
    let api_key = std::env::var("ZAI_API_KEY").unwrap();
    let provider = OpenAIProvider::with_base_url(
        "glm-4.5-air",
        api_key,
        "https://api.z.ai/api/paas/v4/chat/completions"
    );

    let messages = vec![Message::user("Hello")];
    let response = provider.chat(&messages).await.unwrap();

    assert!(!response.content.is_empty());
}
```

### Backward Compatibility

All existing OpenAI tests must pass without modification to ensure zero breaking changes.

## Future Extensibility

This design makes it trivial to add other OpenAI-compatible providers:

```rust
// Together AI
let provider = OpenAIProvider::with_base_url(
    "meta-llama/Llama-3-70b-chat-hf",
    api_key,
    "https://api.together.xyz/v1/chat/completions"
);

// DeepInfra
let provider = OpenAIProvider::with_base_url(
    "meta-llama/Meta-Llama-3-70B-Instruct",
    api_key,
    "https://api.deepinfra.com/v1/openai/chat/completions"
);
```

## Key Benefits

1. **Minimal Changes:** Only 1 field + 1 method added
2. **Zero Breaking Changes:** Existing code works unchanged
3. **Maximum Flexibility:** Easy to extend for multiple providers
4. **Simple Testing:** Clear separation of concerns
5. **Clean Architecture:** Follows existing patterns

## References

- [Z.AI API Introduction](https://docs.z.ai/api-reference/introduction)
- [Z.AI HTTP API Calls](https://docs.z.ai/guides/develop/http/introduction)
- [OpenAI API Documentation](https://platform.openai.com/docs/api-reference)

## Next Steps

After this design is approved:
1. Invoke `writing-plans` skill to create detailed implementation plan
2. Execute implementation
3. Run tests
4. Update documentation
