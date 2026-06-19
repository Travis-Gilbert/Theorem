# Theorem Development Patterns

> Auto-generated skill from repository analysis

## Overview
This skill teaches the core development patterns and conventions used in the Theorem Rust codebase. It covers file organization, code style, commit practices, and testing approaches, providing clear examples and step-by-step workflows to help you contribute effectively.

## Coding Conventions

### File Naming
- Use **camelCase** for file names.
  - Example:  
    ```plaintext
    theoremCore.rs
    proofEngine.rs
    ```

### Import Style
- Use **relative imports** within the codebase.
  - Example:
    ```rust
    mod utils;
    use crate::utils::mathHelpers;
    ```

### Export Style
- Use **named exports** for modules and functions.
  - Example:
    ```rust
    pub fn prove_theorem() { ... }
    pub mod logic;
    ```

### Commit Messages
- Follow **conventional commits** with the `feat` prefix for features.
- Keep commit messages concise (average 75 characters).
  - Example:
    ```
    feat: add support for new proof strategy in theorem engine
    ```

## Workflows

### Adding a New Feature
**Trigger:** When implementing a new functionality  
**Command:** `/add-feature`

1. Create a new file using camelCase naming if needed.
2. Write the feature code, using relative imports and named exports.
3. Add or update tests in a corresponding `*.test.*` file.
4. Commit changes with a message like:  
   `feat: describe the new feature briefly`
5. Push your branch and open a pull request.

### Writing Tests
**Trigger:** When adding or updating tests  
**Command:** `/write-test`

1. Create or update a test file matching the `*.test.*` pattern.
2. Write test cases for your module or function.
3. Run tests using the Rust test runner (e.g., `cargo test`).
4. Ensure all tests pass before committing.

## Testing Patterns

- Test files follow the `*.test.*` naming pattern (e.g., `logic.test.rs`).
- The specific test framework is not specified; use Rust's built-in test framework unless otherwise noted.
- Example test structure:
  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn test_prove_theorem() {
          assert!(prove_theorem());
      }
  }
  ```

## Commands
| Command       | Purpose                                    |
|---------------|--------------------------------------------|
| /add-feature  | Guide for adding a new feature             |
| /write-test   | Steps for writing and running tests        |
