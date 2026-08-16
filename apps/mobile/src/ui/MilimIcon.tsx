import React from 'react';
import Svg, {Circle, Path, Rect} from 'react-native-svg';

export type MilimIconName =
  | 'archive'
  | 'arrow-left'
  | 'arrow-up'
  | 'bolt'
  | 'camera'
  | 'check'
  | 'chevron-down'
  | 'chevron-right'
  | 'chevron-up'
  | 'file'
  | 'folder'
  | 'image'
  | 'info'
  | 'link'
  | 'more-horizontal'
  | 'paperclip'
  | 'pencil'
  | 'plus'
  | 'refresh'
  | 'scan'
  | 'search'
  | 'sidebar'
  | 'smartphone'
  | 'sparkles'
  | 'square'
  | 'trash'
  | 'x';

export function MilimIcon({
  name,
  size = 18,
  color = 'currentColor',
}: {
  name: MilimIconName;
  size?: number;
  color?: string;
}) {
  const common = {
    fill: 'none',
    stroke: color,
    strokeLinecap: 'round' as const,
    strokeLinejoin: 'round' as const,
    strokeWidth: 1.8,
  };

  return (
    <Svg width={size} height={size} viewBox="0 0 24 24" accessibilityElementsHidden>
      {name === 'archive' ? (
        <>
          <Path d="M4 4h16v5H4Z" {...common} />
          <Path d="M6 9v9a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2V9M10 13h4" {...common} />
        </>
      ) : null}
      {name === 'arrow-left' ? <Path d="M19 12H5M11 6l-6 6 6 6" {...common} /> : null}
      {name === 'arrow-up' ? <Path d="M12 19V5M6 11l6-6 6 6" {...common} /> : null}
      {name === 'bolt' ? <Path d="M13 2 4 14h6l-1 8 9-12h-6l1-8Z" fill={color} /> : null}
      {name === 'camera' ? (
        <>
          <Path d="M14.5 5 13 3H8L6.5 5H5a2 2 0 0 0-2 2v11a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2Z" {...common} />
          <Circle cx="11" cy="12" r="3.5" {...common} />
        </>
      ) : null}
      {name === 'check' ? <Path d="m5 12 5 5 9-10" {...common} /> : null}
      {name === 'chevron-down' ? <Path d="m6 9 6 6 6-6" {...common} /> : null}
      {name === 'chevron-right' ? <Path d="m9 6 6 6-6 6" {...common} /> : null}
      {name === 'chevron-up' ? <Path d="m6 15 6-6 6 6" {...common} /> : null}
      {name === 'file' ? (
        <>
          <Path d="M14 3v4a1 1 0 0 0 1 1h4" {...common} />
          <Path d="M17 21H7a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h7l5 5v11a2 2 0 0 1-2 2ZM9 13h6M9 17h6" {...common} />
        </>
      ) : null}
      {name === 'folder' ? <Path d="M3 7a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z" {...common} /> : null}
      {name === 'image' ? (
        <>
          <Rect x="3" y="5" width="18" height="14" rx="2" {...common} />
          <Circle cx="8.5" cy="10" r="1.5" {...common} />
          <Path d="m21 15-4.5-4.5L9 18" {...common} />
        </>
      ) : null}
      {name === 'info' ? (
        <>
          <Circle cx="12" cy="12" r="9" {...common} />
          <Path d="M12 11v5" {...common} />
          <Circle cx="12" cy="8" r="1" fill={color} />
        </>
      ) : null}
      {name === 'link' ? <Path d="M10 13a5 5 0 0 0 7.5.5l2-2a5 5 0 0 0-7-7l-1.2 1.2M14 11a5 5 0 0 0-7.5-.5l-2 2a5 5 0 0 0 7 7l1.2-1.2" {...common} /> : null}
      {name === 'more-horizontal' ? (
        <>
          <Circle cx="5" cy="12" r="1.25" fill={color} />
          <Circle cx="12" cy="12" r="1.25" fill={color} />
          <Circle cx="19" cy="12" r="1.25" fill={color} />
        </>
      ) : null}
      {name === 'paperclip' ? <Path d="M21 11.5 12.5 20a4.5 4.5 0 0 1-6.4-6.4l8.5-8.5a3 3 0 0 1 4.3 4.3l-8.6 8.5a1.5 1.5 0 0 1-2.1-2.1l7.8-7.8" {...common} /> : null}
      {name === 'pencil' ? <Path d="M12 20h9M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z" {...common} /> : null}
      {name === 'plus' ? <Path d="M12 5v14M5 12h14" {...common} /> : null}
      {name === 'refresh' ? <Path d="M21 12a9 9 0 1 1-2.6-6.4M21 4v5h-5" {...common} /> : null}
      {name === 'scan' ? <Path d="M4 8V5a1 1 0 0 1 1-1h3M16 4h3a1 1 0 0 1 1 1v3M20 16v3a1 1 0 0 1-1 1h-3M8 20H5a1 1 0 0 1-1-1v-3M8 12h8" {...common} /> : null}
      {name === 'search' ? (
        <>
          <Circle cx="10.5" cy="10.5" r="6.5" {...common} />
          <Path d="m16 16 4.5 4.5" {...common} />
        </>
      ) : null}
      {name === 'sidebar' ? (
        <>
          <Rect x="3" y="4" width="18" height="16" rx="2" {...common} />
          <Path d="M9 4v16" {...common} />
        </>
      ) : null}
      {name === 'smartphone' ? (
        <>
          <Rect x="7" y="2" width="10" height="20" rx="2" {...common} />
          <Path d="M11 18h2" {...common} />
        </>
      ) : null}
      {name === 'sparkles' ? <Path d="M12 4l1.4 3.6L17 9l-3.6 1.4L12 14l-1.4-3.6L7 9l3.6-1.4ZM19 14l.7 1.8L21.5 16l-1.8.7L19 18.5l-.7-1.8L16.5 16l1.8-.5Z" {...common} /> : null}
      {name === 'square' ? <Rect x="6" y="6" width="12" height="12" rx="2.5" fill={color} /> : null}
      {name === 'trash' ? <Path d="M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2M6 7l1 13a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1l1-13" {...common} /> : null}
      {name === 'x' ? <Path d="M6 6l12 12M18 6 6 18" {...common} /> : null}
    </Svg>
  );
}
