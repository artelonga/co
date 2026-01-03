---
type: task
id: "35.1"
story: "[[35]]"
status: done
github: 35
language: english
---

# Implement Extensible Content Types

## Given
the acceptance criteria from issue #35:
- User can create `<type>/schema.yaml` to define new content type
- `co new <type> <name>` creates content from any registered type
- Built-in types: epic, user-story, task, use-case
- Validation respects custom schemas
- `co schema list` shows all available types

## When
the implementation is complete

## Then
- FeatureRegistry discovers content types from `<type>/schema.yaml`
- Validation uses discovered types instead of hardcoded KNOWN_TYPES
- `co new` validates type exists in registered schemas
- Built-in schemas exist for epic, user-story, task, use-case
- `co schema list` shows all content types

---

## Implementation Plan

### Phase 1: Integrate FeatureRegistry with Validation
- [ ] Add method to FeatureRegistry to check if a content type exists
- [ ] Modify ValidationContext to optionally use FeatureRegistry
- [ ] Update validate_file to use discovered types

### Phase 2: Support Content Type Schemas
- [ ] Create built-in schemas for epic, user-story, task, use-case
- [ ] Ensure work/schema.yaml structure supports content types
- [ ] Update `co schema list` to show content types

### Phase 3: Schema-aware `co new`
- [ ] Validate content type exists before creating file
- [ ] Generate appropriate template based on schema

### Phase 4: Tests
- [ ] Test that custom types are discovered
- [ ] Test that validation accepts discovered types
- [ ] Test that `co new` rejects unknown types
