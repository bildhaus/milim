import {isLoadableImageSource} from '../src/transcript/imageSources';

test('web and inline images can be shown on the phone', () => {
  expect(isLoadableImageSource('https://example.com/a.png')).toBe(true);
  expect(isLoadableImageSource('http://desk.tailnet.ts.net:10000/x.webp')).toBe(true);
  expect(isLoadableImageSource('data:image/png;base64,AAAA')).toBe(true);
});

test('desktop-only sources are reported instead of failing to load', () => {
  expect(isLoadableImageSource('/Users/me/Pictures/out.png')).toBe(false);
  expect(isLoadableImageSource('file:///tmp/out.png')).toBe(false);
  expect(isLoadableImageSource('http://localhost:7377/media/1')).toBe(false);
  expect(isLoadableImageSource('http://127.0.0.1:7377/media/1')).toBe(false);
  expect(isLoadableImageSource('http://[::1]:7377/media/1')).toBe(false);
  expect(isLoadableImageSource('data:text/html;base64,AAAA')).toBe(false);
  expect(isLoadableImageSource('')).toBe(false);
});
