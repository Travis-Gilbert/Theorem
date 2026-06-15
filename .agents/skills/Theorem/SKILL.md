```markdown
# Theorem Development Patterns

> Auto-generated skill from repository analysis

## Overview
This skill teaches the core development patterns and conventions used in the Theorem Rust codebase. You'll learn how to structure files, write imports and exports, follow commit message conventions, and organize tests. This guide also provides commands for common workflows to streamline your development process.

## Coding Conventions

### File Naming
- Use **camelCase** for file names.
  - Example: `myModule.rs`, `mathUtils.rs`

### Import Style
- Use **relative imports** within the codebase.
  - Example:
    ```rust
    mod mathUtils;
    use crate::mathUtils::add;
    ```

### Export Style
- Use **named exports** for modules and functions.
  - Example:
    ```rust
    pub fn calculate_sum(a: i32, b: i32) -> i32 {
        a + b
    }
    ```

### Commit Messages
- Follow **conventional commit** format.
- Use the `feat` prefix for new features.
  - Example: `feat: add theorem prover for group theory`
- Average commit message length: ~62 characters.

## Workflows

### Creating a New Feature
**Trigger:** When adding a new feature to the codebase  
**Command:** `/new-feature`

1. Create a new file using camelCase naming.
2. Implement your feature using relative imports for dependencies.
3. Export your functions or modules with `pub`.
4. Write corresponding tests in a `*.test.*` file.
5. Commit your changes using the `feat:` prefix and a descriptive message.

### Adding Tests
**Trigger:** When writing or updating tests for a module  
**Command:** `/add-test`

1. Create a test file with the pattern `*.test.*` (e.g., `mathUtils.test.rs`).
2. Write tests using Rust's built-in `#[test]` framework.
   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;

       #[test]
       fn test_calculate_sum() {
           assert_eq!(calculate_sum(2, 3), 5);
       }
   }
   ```
3. Run tests with `cargo test`.

### Making a Commit
**Trigger:** When committing code changes  
**Command:** `/commit`

1. Stage your changes with `git add`.
2. Write a commit message starting with `feat:`, followed by a concise description.
   - Example: `feat: implement basic theorem validation logic`
3. Commit with `git commit -m "feat: your message here"`

## Testing Patterns

- Test files follow the `*.test.*` pattern (e.g., `module.test.rs`).
- Use Rust's built-in test framework with `#[test]` annotations.
- Place tests in the same file or in a separate test file as appropriate.
- Example:
  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;

      #[test]
      fn test_functionality() {
          // test code here
      }
  }
  ```

## Commands
| Command       | Purpose                                 |
|---------------|-----------------------------------------|
| /new-feature  | Scaffold a new feature module           |
| /add-test     | Add or update tests for a module        |
| /commit       | Make a conventional commit              |
```
