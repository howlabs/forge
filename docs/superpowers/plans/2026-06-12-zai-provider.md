# Z.AI Provider Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Z.AI (international) provider support to forge CLI by extending OpenAIProvider with configurable base URL.

**Architecture:** Extend `OpenAIProvider` struct with optional `base_url` field and add `with_base_url()` constructor. Update `chat()` method to use custom endpoint when provided, defaulting to OpenAI endpoint otherwise.

**Tech Stack:** Rust, async/await, reqwest HTTP client, anyhow error handling, serde JSON.

---

## File Structure

**Files to modify:**
- `forge-provider/src/openai.rs` - Main implementation file
  - Add `base_url` field to struct
  - Add `with_base_url()` constructor
  - Update `chat()` method to use dynamic URL

**No new files needed** - we're extending existing functionality.

---

## Task 1: Add `base_url` field to `OpenAIProvider` struct

**Files:**
- Modify: `forge-provider/src/openai.rs:14-18`

- [ ] **Step 1: Read current struct definition**

Run: `cat forge-provider/src/openai.rs | head -20`

Expected output showing current struct:
```rust
pub struct OpenAIProvider {
    model: String,
    api_key: String,
    client: Client,
}
```

- [ ] **Step 2: Add `base_url` field to struct**

Replace the struct definition with:

```rust
pub struct OpenAIProvider {
    model: String,
    api_key: String,
    client: Client,
    base_url: Option<String>,
}
```

- [ ] **Step 3: Run cargo check to verify compilation**

Run: `cargo check --package forge-provider`

Expected: Compilation error (constructors don't include new field)

- [ ] **Step 4: Commit struct change**

```bash
git add forge-provider/src/openai.rs
git commit -m "feat(provider): add base_url field to OpenAIProvider"
```

---

## Task 2: Update existing `new()` constructor to initialize `base_url`

**Files:**
- Modify: `forge-provider/src/openai.rs:36-43`

- [ ] **Step 1: Read current constructor**

Run: `sed -n '36,43p' forge-provider/src/openai.rs`

Expected:
```rust
pub fn new(model: impl Into<String>, api_key: impl Into<String>) -> Self {
    Self {
        model: model.into(),
        api_key: api_key.into(),
        client: Client::new(),
    }
}
```

- [ ] **Step 2: Update constructor to initialize base_url**

Replace with:

```rust
pub fn new(model: impl Into<String>, api_key: impl Into<String>) -> Self {
    Self {
        model: model.into(),
        api_key: api_key.into(),
        client: Client::new(),
        base_url: None,
    }
}
```

- [ ] **Step 3: Run cargo check**

Run: `cargo check --package forge-provider`

Expected: Compiles successfully

- [ ] **Step 4: Commit constructor update**

```bash
git add forge-provider/src/openai.rs
git commit -m "feat(provider): update OpenAIProvider::new to init base_url"
```

---

## Task 3: Add `with_base_url()` constructor for custom endpoints

**Files:**
- Modify: `forge-provider/src/openai.rs:44-52` (insert after `new()` method)

- [ ] **Step 1: Add new constructor after existing `new()` method**

Insert this code after line 43 (after the closing brace of `new()`):

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

- [ ] **Step 2: Run cargo check**

Run: `cargo check --package forge-provider`

Expected: Compiles successfully

- [ ] **Step 3: Commit new constructor**

```bash
git add forge-provider/src/openai.rs
git commit -m "feat(provider): add with_base_url constructor for custom endpoints"
```

---

## Task 4: Write test for `with_base_url()` constructor

**Files:**
- Modify: `forge-provider/src/openai.rs` (add to tests module at end of file)

- [ ] **Step 1: Find tests module location**

Run: `grep -n "#\[cfg(test)\]" forge-provider/src/openai.rs`

Expected: Line number around 101

- [ ] **Step 2: Add test for with_base_url constructor**

Add this test inside the `mod tests` block:

```rust
#[test]
fn test_with_base_url() {
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

- [ ] **Step 3: Run the test**

Run: `cargo test --package forge-provider test_with_base_url -- --nocapture`

Expected: FAIL with "no field `base_url`" (it's a private field)

- [ ] **Step 4: Make base_url accessible for testing**

We need to add a getter method. Add this public method after the `model()` getter:

```rust
pub fn base_url(&self) -> Option<&str> {
    self.base_url.as_deref()
}
```

- [ ] **Step 5: Run test again**

Run: `cargo test --package forge-provider test_with_base_url -- --nocapture`

Expected: PASS

- [ ] **Step 6: Commit test and getter**

```bash
git add forge-provider/src/openai.rs
git commit -m "feat(provider): add base_url getter and test"
```

---

## Task 5: Update `chat()` method to use dynamic URL

**Files:**
- Modify: `forge-provider/src/openai.rs:61-74`

- [ ] **Step 1: Read current chat() method URL usage**

Run: `sed -n '61,74p' forge-provider/src/openai.rs`

Expected:
```rust
async fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
    let openai_messages = Self::convert_messages(messages);

    let response = self
        .client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", self.api_key))
        .header("Content-Type", "application/json")
        .json(&json!({
            "model": self.model,
            "messages": openai_messages
        }))
        .send()
        .await?;
    // ... error handling continues
```

- [ ] **Step 2: Add URL selection logic before the HTTP request**

Insert this code after line 63 (after `let openai_messages = ...`):

```rust
    let url = self.base_url.as_deref()
        .unwrap_or("https://api.openai.com/v1/chat/completions");
```

- [ ] **Step 3: Replace hardcoded URL with dynamic variable**

Change line 65 from:
```rust
        .post("https://api.openai.com/v1/chat/completions")
```

To:
```rust
        .post(url)
```

The updated section should look like:
```rust
async fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
    let openai_messages = Self::convert_messages(messages);

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
```

- [ ] **Step 4: Run cargo check**

Run: `cargo check --package forge-provider`

Expected: Compiles successfully

- [ ] **Step 5: Commit chat method update**

```bash
git add forge-provider/src/openai.rs
git commit -m "feat(provider): update chat() to use dynamic base URL"
```

---

## Task 6: Add integration test for Z.AI API (optional)

**Files:**
- Modify: `forge-provider/src/openai.rs` (add to tests module)

- [ ] **Step 1: Add integration test**

Add this test to the `mod tests` block:

```rust
#[tokio::test]
#[ignore]
async fn test_zai_real_api() {
    let api_key = std::env::var("ZAI_API_KEY")
        .expect("Set ZAI_API_KEY environment variable to run this test");

    let provider = OpenAIProvider::with_base_url(
        "glm-4.5-air",  // Cheaper model for testing
        api_key,
        "https://api.z.ai/api/paas/v4/chat/completions"
    );

    let messages = vec![Message::user("Say hello in one sentence")];
    let response = provider.chat(&messages).await;

    assert!(response.is_ok());
    let response = response.unwrap();
    assert!(!response.content.is_empty());
    println!("Z.AI Response: {}", response.content);
}
```

- [ ] **Step 2: Test that it's ignored by default**

Run: `cargo test --package forge-provider test_zai_real_api`

Expected: Test is ignored (not run)

- [ ] **Step 3: Document how to run integration test**

Add comment above test:
```rust
// Integration test with real Z.AI API
// Run with: cargo test --package forge-provider test_zai_real_api -- --ignored
// Requires: ZAI_API_KEY environment variable
#[tokio::test]
#[ignore]
async fn test_zai_real_api() {
```

- [ ] **Step 4: Commit integration test**

```bash
git add forge-provider/src/openai.rs
git commit -m "test(provider): add Z.AI integration test"
```

---

## Task 7: Run all tests to verify nothing broke

**Files:**
- Test: `forge-provider/src/openai.rs`

- [ ] **Step 1: Run all provider tests**

Run: `cargo test --package forge-provider`

Expected: All tests pass

- [ ] **Step 2: Run with verbose output**

Run: `cargo test --package forge-provider -- --nocapture`

Expected: All tests pass with detailed output

- [ ] **Step 3: Check that existing tests still pass**

Specifically check the existing tests:
```bash
cargo test --package forge-provider test_openai_provider_creation
cargo test --package forge-provider test_convert_messages
```

Expected: Both pass

- [ ] **Step 4: If all tests pass, commit verification**

```bash
git add .
git commit -m "test(provider): verify all tests pass after changes"
```

---

## Task 8: Create documentation and usage examples

**Files:**
- Create: `docs/providers/zai.md`

- [ ] **Step 1: Create Z.AI provider documentation**

Create file `docs/providers/zai.md`:

```markdown
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

\`\`\`bash
# Set your API key
export ZAI_API_KEY=your_api_key

# Run integration test
cargo test --package forge-provider test_zai_real_api -- --ignored
\`\`\`

## References

- [Z.AI API Documentation](https://docs.z.ai/api-reference/introduction)
- [Z.AI HTTP API Guide](https://docs.z.ai/guides/develop/http/introduction)
```

- [ ] **Step 2: Create docs/providers directory if needed**

Run: `mkdir -p docs/providers`

- [ ] **Step 3: Verify file was created**

Run: `ls -la docs/providers/zai.md`

Expected: File exists

- [ ] **Step 4: Commit documentation**

```bash
git add docs/providers/zai.md
git commit -m "docs: add Z.AI provider documentation"
```

---

## Task 9: Update README with provider information

**Files:**
- Modify: `README.md` (or forge-provider/README.md if exists)

- [ ] **Step 1: Check for README files**

Run: `ls -la README.md forge-provider/README.md 2>/dev/null || echo "No README found"`

- [ ] **Step 2: Add provider list to main README**

If `README.md` exists, add this section:

```markdown
## Supported Providers

Forge CLI supports multiple AI model providers:

- **OpenAI** - GPT-4, GPT-3.5 (default)
- **Anthropic** - Claude models
- **Gemini** - Google Gemini models
- **Local** - Local models
- **Z.AI** - GLM models (glm-5.1, glm-4.7, glm-4.5, glm-4.5-air)

See [docs/providers/](docs/providers/) for detailed documentation.
```

- [ ] **Step 3: Commit README update**

```bash
git add README.md
git commit -m "docs: update README with Z.AI provider information"
```

---

## Task 10: Final verification and cleanup

**Files:**
- All modified files

- [ ] **Step 1: Run full cargo test**

Run: `cargo test --workspace`

Expected: All tests pass across all packages

- [ ] **Step 2: Run cargo check**

Run: `cargo check --workspace`

Expected: No warnings or errors

- [ ] **Step 3: Run cargo clippy**

Run: `cargo clippy --workspace -- -D warnings`

Expected: No clippy warnings

- [ ] **Step 4: Format code**

Run: `cargo fmt --all`

Expected: Code formatted successfully

- [ ] **Step 5: Show git diff**

Run: `git diff HEAD~10..HEAD --stat`

Expected: Shows only our changes

- [ ] **Step 6: Final commit**

```bash
git add .
git commit -m "feat(provider): complete Z.AI provider implementation

- Add configurable base_url to OpenAIProvider
- Add with_base_url() constructor for custom endpoints
- Update chat() method to use dynamic URL
- Add unit tests and integration test
- Add documentation for Z.AI provider

Co-Authored-By: Claude Sonnet 4.6 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review Checklist

**✓ Spec Coverage:**
- ✓ Z.AI API endpoint supported
- ✓ Configurable base URL implemented
- ✓ Backward compatibility maintained
- ✓ Unit tests included
- ✓ Integration test included
- ✓ Documentation added

**✓ Placeholder Scan:**
- ✓ No TBD/TODO items
- ✓ No "add error handling" placeholders
- ✓ All code complete in steps

**✓ Type Consistency:**
- ✓ Field names consistent (`base_url`)
- ✓ Method names consistent (`with_base_url()`, `base_url()`)
- ✓ Types match throughout

**✓ File Structure:**
- ✓ Minimal file modifications
- ✓ Clear responsibility per file
- ✓ Follows existing patterns

---

## Success Criteria

Implementation is complete when:

1. ✅ `OpenAIProvider` has `base_url` field
2. ✅ `with_base_url()` constructor works
3. ✅ `chat()` method uses dynamic URL
4. ✅ All existing tests still pass
5. ✅ New tests pass
6. ✅ Documentation is complete
7. ✅ Code is formatted and linted

---

## Notes

- **Backward Compatibility:** All existing code using `OpenAIProvider::new()` continues to work unchanged
- **Zero Breaking Changes:** The implementation is additive only
- **Testing:** Integration test requires real API key but is ignored by default
- **Future Extensibility:** This pattern can be reused for other OpenAI-compatible providers (Together AI, DeepInfra, etc.)
