import fs from 'node:fs';
import path from 'node:path';

function fixture(name: string): any {
  return JSON.parse(
    fs.readFileSync(path.resolve(__dirname, '../../../contracts/control-v1', name), 'utf8'),
  );
}

test('checked-in v1 fixtures expose stable protocol coordinates', () => {
  const bootstrap = fixture('bootstrap.json');
  const timeline = fixture('timeline.json');
  const command = fixture('command-turn-send.json');
  expect(bootstrap.protocol).toEqual({min: 1, max: 1});
  expect(typeof bootstrap.host_id).toBe('string');
  expect(bootstrap.appearance.theme_id).toBe('fixture-custom');
  expect(bootstrap.appearance.colors.accent).toBe('#38bdf8');
  expect(timeline.items.every((item: any) => item.epoch === timeline.epoch)).toBe(true);
  expect(command.kind).toBe('turn.send');
  expect(typeof command.command_id).toBe('string');
});

test('approval fixture never contains an entered response value', () => {
  const approval = fixture('approval.json');
  expect(JSON.stringify(approval)).not.toContain('response_value');
});

test('generated v1 fixture keeps additive harness capabilities and commands current', () => {
  const generated = JSON.parse(fs.readFileSync(
    path.resolve(__dirname, '../../../crates/milim-control-contract/fixtures/control-v1.json'),
    'utf8',
  ));
  expect(generated.protocol).toEqual({min: 1, max: 1});
  expect(generated.capabilities).toMatchObject({
    run_ledger: true,
    run_inspection: true,
    steering: true,
    context_injection: true,
    model_favorites: true,
  });
  expect(generated.command_kinds).toEqual(expect.arrayContaining([
    'model_favorites.set',
    'turn.steer',
    'context.inject',
    'turn.inbox_delete',
  ]));
});
