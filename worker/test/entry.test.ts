// workerd treats every named export of the main module as an entrypoint and refuses to load a non-function
// one (P7 gate R4-1 — vitest imports TS directly and could not see it). Keep src/index.ts default-only.
import { describe, expect, it } from 'vitest';
import * as entry from '../src/index';

describe('entry module (wrangler.toml main)', () => {
  it('exports only `default`', () => {
    expect(Object.keys(entry)).toEqual(['default']);
  });
  it('default is a handler with scheduled + fetch', () => {
    expect(typeof entry.default.scheduled).toBe('function');
    expect(typeof entry.default.fetch).toBe('function');
  });
});
