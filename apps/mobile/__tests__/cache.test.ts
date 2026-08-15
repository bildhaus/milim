const mockCalls: string[] = [];
const mockExecute = jest.fn(async (sql: string) => {
  mockCalls.push(`execute:${sql}`);
  return {rows: []};
});
const mockExecuteBatch = jest.fn(async () => {
  mockCalls.push('batch');
});

jest.mock('@op-engineering/op-sqlite', () => ({
  open: () => ({execute: mockExecute, executeBatch: mockExecuteBatch}),
}));

const {listHosts} = require('../src/storage/cache') as typeof import('../src/storage/cache');

test('enables WAL before the schema batch opens its transaction', async () => {
  await listHosts();

  expect(mockCalls[0]).toBe('execute:PRAGMA journal_mode = WAL');
  expect(mockCalls[1]).toBe('batch');
  expect(mockCalls[2]).toContain('SELECT metadata_json FROM hosts');
});
