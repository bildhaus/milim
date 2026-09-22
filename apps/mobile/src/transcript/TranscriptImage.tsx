import React, {useState} from 'react';
import {Image, Modal, Platform, Pressable, ScrollView, Share, StyleSheet, View, useWindowDimensions} from 'react-native';
import {SafeAreaView} from 'react-native-safe-area-context';
import {useAppTheme, Text} from '../ui/appTheme';
import {IconButton} from '../ui/controls';
import {showError} from '../ui/dialogs';
import {isLoadableImageSource} from './imageSources';

export function TranscriptImage({source, alt}: {source: string; alt: string}) {
  const {styles} = useAppTheme();
  const [aspectRatio, setAspectRatio] = useState(4 / 3);
  const [failed, setFailed] = useState(false);
  const [viewing, setViewing] = useState(false);

  if (failed || !isLoadableImageSource(source)) {
    return (
      <View style={styles.transcriptImageUnavailable} accessibilityRole="text">
        <Text style={styles.transcriptImageUnavailableText}>
          {alt ? `Image: ${alt}` : 'Image'} · available on desktop
        </Text>
      </View>
    );
  }
  return (
    <>
      <Pressable
        onPress={() => setViewing(true)}
        accessibilityRole="imagebutton"
        accessibilityLabel={alt ? `${alt}. Open image` : 'Open image'}>
        <Image
          source={{uri: source}}
          style={[styles.transcriptImage, {aspectRatio}]}
          resizeMode="contain"
          onLoad={({nativeEvent}) => {
            const {width, height} = nativeEvent.source;
            if (width > 0 && height > 0) setAspectRatio(Math.max(0.4, Math.min(3, width / height)));
          }}
          onError={() => setFailed(true)}
        />
      </Pressable>
      {viewing ? <ImageViewer source={source} alt={alt} onClose={() => setViewing(false)} /> : null}
    </>
  );
}

function ImageViewer({source, alt, onClose}: {source: string; alt: string; onClose: () => void}) {
  const {styles} = useAppTheme();
  const {width, height} = useWindowDimensions();
  const shareable = !source.startsWith('data:');
  return (
    <Modal visible animationType="fade" statusBarTranslucent onRequestClose={onClose}>
      <View style={styles.imageViewer}>
        {/* Pinch and double-tap zoom come from the native scroll view; Android
            shows the image fitted, where ScrollView has no zoom support. */}
        <ScrollView
          style={StyleSheet.absoluteFill}
          contentContainerStyle={styles.imageViewerContent}
          maximumZoomScale={Platform.OS === 'ios' ? 5 : 1}
          minimumZoomScale={1}
          bouncesZoom
          centerContent
          showsHorizontalScrollIndicator={false}
          showsVerticalScrollIndicator={false}>
          <Image
            source={{uri: source}}
            style={{width, height}}
            resizeMode="contain"
            accessibilityLabel={alt || 'Image'}
          />
        </ScrollView>
        <SafeAreaView style={styles.imageViewerBar} edges={['top', 'left', 'right']} pointerEvents="box-none">
          <IconButton icon="x" label="Close image" onPress={onClose} />
          {shareable ? (
            <IconButton
              icon="share"
              label="Share image link"
              onPress={() => void Share.share(Platform.OS === 'ios' ? {url: source} : {message: source}).catch(showError)}
            />
          ) : null}
        </SafeAreaView>
      </View>
    </Modal>
  );
}
