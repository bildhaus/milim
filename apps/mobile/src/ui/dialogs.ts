import {Alert} from 'react-native';
import {type ControlCommandV1, type JsonValue} from '../control/types';
import {useMilimController} from '../controller/useMilimController';

export function modelId(value: JsonValue): string | null {
  if (typeof value === 'string') return value;
  if (typeof value === 'object' && value !== null && !Array.isArray(value)) {
    const record = value as Record<string, JsonValue | undefined>;
    if (typeof record.id === 'string') return record.id;
  }
  return null;
}

export async function confirmDestructive(execute: ReturnType<typeof useMilimController>['execute'], command: ControlCommandV1) {
  try {
    const challenge = await execute(command);
    if (challenge.status !== 'needs_confirmation' || !challenge.confirmation_token) return;
    Alert.alert('Confirm destructive action', challenge.message ?? 'This cannot be undone.', [
      {text: 'Cancel', style: 'cancel'},
      {
        text: 'Confirm',
        style: 'destructive',
        onPress: () => void execute({...command, confirmation_token: challenge.confirmation_token}).catch(showError),
      },
    ]);
  } catch (error) {
    showError(error);
  }
}

export function showError(error: unknown) {
  Alert.alert('milim', error instanceof Error ? error.message : String(error));
}
