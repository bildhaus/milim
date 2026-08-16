declare module '@react-native-masked-view/masked-view' {
  import type {ComponentType, ReactElement} from 'react';
  import type {ViewProps} from 'react-native';

  interface MaskedViewProps extends ViewProps {
    maskElement: ReactElement;
    androidRenderingMode?: 'software' | 'hardware';
  }

  const MaskedView: ComponentType<MaskedViewProps>;
  export default MaskedView;
}
