jest.mock('react-native-blob-util', () => ({
  __esModule: true,
  default: {
    fs: {readFile: jest.fn()},
    fetch: jest.fn(),
    wrap: jest.fn((path: string) => `wrapped:${path}`),
  },
}));
jest.mock('@react-native-documents/picker', () => ({pick: jest.fn(), keepLocalCopy: jest.fn()}));
jest.mock('react-native-image-picker', () => ({
  launchCamera: jest.fn(),
  launchImageLibrary: jest.fn(),
}));
jest.mock('../src/storage/cache', () => ({
  forgetTemporaryFile: jest.fn(),
  staleTemporaryFiles: jest.fn(),
  trackTemporaryFile: jest.fn(),
}));

import {prepareWireAttachments} from '../src/attachments';
import ReactNativeBlobUtil from 'react-native-blob-util';

const mockReadFile = ReactNativeBlobUtil.fs.readFile as jest.Mock;
const mockFetchBlob = ReactNativeBlobUtil.fetch as jest.Mock;
const mockWrap = ReactNativeBlobUtil.wrap as jest.Mock;

const binary = {
  id: 'attachment-1',
  name: 'pixel.png',
  mime: 'image/png',
  size: 3,
  local_uri: 'file:///cache/pixel.png',
};

beforeEach(() => {
  jest.clearAllMocks();
});

test('streams supported binary attachments without reading base64 into JavaScript', async () => {
  mockFetchBlob.mockResolvedValue({
    info: () => ({status: 200}),
    json: () => ({upload_id: 'upload-1', expires_at_ms: Date.now() + 1_000}),
  });

  await expect(prepareWireAttachments([binary], {
    endpoint: 'http://desktop.test',
    deviceKey: 'secret',
    uploads: true,
  })).resolves.toEqual([{
    id: 'attachment-1',
    name: 'pixel.png',
    mime: 'image/png',
    size: 3,
    upload_id: 'upload-1',
  }]);
  expect(mockReadFile).not.toHaveBeenCalled();
  expect(mockWrap).toHaveBeenCalledWith('/cache/pixel.png');
  expect(mockFetchBlob).toHaveBeenCalledWith(
    'PUT',
    expect.stringContaining('/control/v1/attachments/attachment-1?'),
    expect.objectContaining({Authorization: 'Bearer secret', 'Content-Type': 'image/png'}),
    'wrapped:/cache/pixel.png',
  );
});

test('encodes binary attachments only at send time for older desktops', async () => {
  mockReadFile.mockResolvedValue('AQID');
  await expect(prepareWireAttachments([binary], {
    endpoint: 'http://desktop.test',
    deviceKey: 'secret',
    uploads: false,
  })).resolves.toEqual([{
    id: 'attachment-1',
    name: 'pixel.png',
    mime: 'image/png',
    size: 3,
    data_url: 'data:image/png;base64,AQID',
  }]);
  expect(mockFetchBlob).not.toHaveBeenCalled();
  expect(mockReadFile).toHaveBeenCalledWith('/cache/pixel.png', 'base64');
});
