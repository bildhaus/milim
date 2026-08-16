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
