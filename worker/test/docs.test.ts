// The published integration guide (skills/lobby-integration/SKILL.md) must stay in sync with the IDL.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

interface IdlAccount { name: string; signer?: boolean; writable?: boolean; optional?: boolean }
const idl = JSON.parse(readFileSync(new URL('../../idls/lobby.json', import.meta.url), 'utf8')) as {
  instructions: { name: string; discriminator: number[]; accounts: IdlAccount[] }[];
  errors: { code: number; name: string }[];
};
const skill = readFileSync(new URL('../../skills/lobby-integration/SKILL.md', import.meta.url), 'utf8');
const hex = (a: number[]) => a.map((b) => b.toString(16).padStart(2, '0')).join('');
const ix = (n: string) => idl.instructions.find((i) => i.name === n)!;
const section = (from: string, to: string) => skill.slice(skill.indexOf(from), skill.indexOf(to));

describe('SKILL.md matches idls/lobby.json', () => {
  it('relayer table: discriminator · account count · payer index of every listed instruction', () => {
    const rows = [...skill.matchAll(/^\s*\| `([a-z_]+)` \| `([0-9a-f]{16})` \| (\d+)[^|]*\| (\d+|—) \|/gm)];
    expect(new Set(rows.map((r) => r[1])).size).toBe(15);
    for (const [, name, disc, count, payer] of rows) {
      const i = ix(name);
      expect(hex(i.discriminator), name).toBe(disc);
      // tick: + the user as trailing read-only signer (relayer crank rule)
      expect(Number(count), name).toBe(i.accounts.length + (name === 'tick' ? 1 : 0));
      const p = i.accounts.findIndex((a) => a.name === 'payer');
      expect(payer === '—' ? -1 : Number(payer), name).toBe(p);
    }
  });

  it('error tables list every code once, with its IDL name · races = 6000–6003 only', () => {
    const rows = [...skill.matchAll(/\| (60\d\d) \| `?([A-Za-z]+)`? /g)].map((m) => [Number(m[1]), m[2]] as const);
    expect(rows.length).toBe(idl.errors.length);
    const listed = new Map(rows);
    for (const e of idl.errors) expect(listed.get(e.code), String(e.code)).toBe(e.name);
    const races = [...section('**Possible races**', '**Not retryable:**').matchAll(/\| (60\d\d) \|/g)].map((m) => Number(m[1]));
    expect(races).toEqual([6000, 6001, 6002, 6003]);
  });

  it('account lists in §3.1: IDL order and (s)/(w)/? flags', () => {
    const seen = new Set<string>();
    for (const line of section('### 3.1', '## 4.').split('\n')) {
      const m = /^([a-z_ /]+?)\s{2,}(.+)$/.exec(line);
      if (!m) continue;
      const accs = m[2].split('·').map((a) => a.split('←')[0].trim());
      for (const n of m[1].split('/').map((x) => x.trim())) {
        const i = idl.instructions.find((x) => x.name === n);
        if (!i) continue;
        expect(seen.has(n), `${n} listed twice`).toBe(false);
        seen.add(n);
        const expected = i.accounts.map((a) => {
          const flags = [a.signer && 's', a.writable && 'w'].filter(Boolean).join(',');
          return `${a.name}${a.optional ? '?' : ''}${flags ? `(${flags})` : ''}`;
        });
        expect(accs, n).toEqual(expected);
      }
    }
    expect([...seen].sort()).toEqual(idl.instructions.map((i) => i.name).sort());
  });
});
