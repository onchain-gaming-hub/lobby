// The published integration guide (skills/lobby-integration/SKILL.md) must stay in sync with the IDL.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const idl = JSON.parse(readFileSync(new URL('../../idls/lobby.json', import.meta.url), 'utf8')) as {
  instructions: { name: string; discriminator: number[]; accounts: { name: string }[] }[];
  errors: { code: number; name: string }[];
};
const skill = readFileSync(new URL('../../skills/lobby-integration/SKILL.md', import.meta.url), 'utf8');
const hex = (a: number[]) => a.map((b) => b.toString(16).padStart(2, '0')).join('');

describe('SKILL.md matches idls/lobby.json', () => {
  it('relayer table: discriminator + account count of every listed instruction', () => {
    const rows = [...skill.matchAll(/^\s*\| `([a-z_]+)` \| `([0-9a-f]{16})` \| (\d+)/gm)];
    expect(rows.length).toBe(15);
    for (const [, name, disc, count] of rows) {
      const ix = idl.instructions.find((i) => i.name === name)!;
      expect(hex(ix.discriminator), name).toBe(disc);
      // tick: + the user as trailing read-only signer (relayer crank rule)
      expect(Number(count), name).toBe(ix.accounts.length + (name === 'tick' ? 1 : 0));
    }
  });

  it('error tables list every code with its IDL name', () => {
    const listed = new Map([...skill.matchAll(/\| (60\d\d) \| `?([A-Za-z]+)`? /g)].map((m) => [Number(m[1]), m[2]]));
    expect(listed.size).toBe(idl.errors.length);
    for (const e of idl.errors) expect(listed.get(e.code), String(e.code)).toBe(e.name);
  });

  it('account lists in §3.1 follow IDL order', () => {
    const block = skill.slice(skill.indexOf('### 3.1'), skill.indexOf('## 4.'));
    let checked = 0;
    for (const line of block.split('\n')) {
      const m = /^([a-z_ /]+?)\s{2,}(.+)$/.exec(line);
      if (!m) continue;
      const accs = m[2].split('·').map((a) => a.split('←')[0].replace(/[?(].*$/, '').trim());
      for (const n of m[1].split('/').map((x) => x.trim())) {
        const ix = idl.instructions.find((i) => i.name === n);
        if (!ix) continue;
        expect(accs, n).toEqual(ix.accounts.map((a) => a.name));
        checked++;
      }
    }
    expect(checked).toBe(idl.instructions.length);
  });
});
