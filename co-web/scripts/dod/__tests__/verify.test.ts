/**
 * CO-443: tests for the DoD verifier proof engine.
 *
 * Covers the refactor/structural matching added on top of the e2e-spec
 * matcher: Rust test recognition, structural proof directives (grep /
 * grep-absent / rust-test / file), frontmatter dod_checks parsing, and the
 * end-to-end requirement that CO-431..436 score >= 80% with no override.
 */

import { describe, it, expect } from 'vitest';
import {
  derivePattern,
  evaluateProof,
  extractRustTests,
  parseDodChecks,
  matchProofsForItem,
  parseSpecFile,
  buildReport,
} from '../verify.ts';

describe('extractRustTests', () => {
  it('finds #[test] and #[tokio::test] functions, skips plain fns', () => {
    const src = [
      '#[test]',
      'fn plain_unit_test() {}',
      '',
      '#[tokio::test]',
      'async fn async_thing_works() {}',
      '',
      'fn not_a_test() {}',
      '',
      '    #[test]',
      '    fn indented_test() {}',
    ].join('\n');
    const names = extractRustTests(src).map(t => t.name);
    expect(names).toContain('plain_unit_test');
    expect(names).toContain('async_thing_works');
    expect(names).toContain('indented_test');
    expect(names).not.toContain('not_a_test');
  });
});

describe('evaluateProof', () => {
  it('grep passes when the pattern exists in the tree', () => {
    const r = evaluateProof('grep:pub trait Universo@core/src/universo.rs');
    expect(r.ok).toBe(true);
  });

  it('grep fails when the pattern is absent', () => {
    const r = evaluateProof('grep:trait ThisTraitDoesNotExistAnywhere@core/src');
    expect(r.ok).toBe(false);
  });

  it('grep-absent passes when the pattern is absent in scope', () => {
    const r = evaluateProof('grep-absent:^axum@game-core/Cargo.toml');
    expect(r.ok).toBe(true);
  });

  it('rust-test passes for a real test function name', () => {
    const r = evaluateProof('rust-test:fresh_db_reaches_latest_version');
    expect(r.ok).toBe(true);
  });

  it('rust-test fails for an unknown function name', () => {
    const r = evaluateProof('rust-test:no_such_test_function_exists');
    expect(r.ok).toBe(false);
  });

  it('file passes when the path exists', () => {
    const r = evaluateProof('file:co-web/src/storage/migrations/mod.rs');
    expect(r.ok).toBe(true);
  });
});

describe('parseDodChecks + matchProofsForItem', () => {
  const fm = [
    'id: 999',
    'dod_checks:',
    '  - "trait Foo exists => grep:trait Foo@core/src"',
    '  - "axum-free => grep-absent:axum@game-core/Cargo.toml && rust-test:something"',
    'module: co-web',
  ].join('\n');

  it('parses each "<match> => <proof>" entry from frontmatter', () => {
    const checks = parseDodChecks(fm);
    expect(checks).toHaveLength(2);
    expect(checks[0].match).toBe('trait Foo exists');
    expect(checks[0].proofs).toEqual(['grep:trait Foo@core/src']);
    expect(checks[1].proofs).toEqual([
      'grep-absent:axum@game-core/Cargo.toml',
      'rust-test:something',
    ]);
  });

  it('matches a check to an acceptance item by substring', () => {
    const checks = parseDodChecks(fm);
    const proofs = matchProofsForItem('the trait Foo exists in core now', checks);
    expect(proofs).toEqual(['grep:trait Foo@core/src']);
  });
});

describe('derivePattern (existing behaviour preserved)', () => {
  it('extracts kebab ids and keywords', () => {
    const p = derivePattern('btn-compor opens compose modal');
    expect(p).toMatch(/btn/);
    expect(p).toMatch(/compose|modal/);
  });
});

describe('CO-431..436 acceptance (>= 80% without override)', () => {
  for (const id of ['CO-431', 'CO-432', 'CO-433', 'CO-434', 'CO-435', 'CO-436']) {
    it(`${id} scores >= 80%`, () => {
      const { title, items, dodChecks } = parseSpecFile(id);
      const report = buildReport(id, title, items, dodChecks);
      expect(report.dod_pct).toBeGreaterThanOrEqual(80);
      expect(report.blocking_failures).toBe(0);
    });
  }
});
