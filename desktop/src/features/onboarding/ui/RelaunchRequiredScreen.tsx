import { RecoveryScreen } from "./RecoveryScreen";

export function RelaunchRequiredScreen() {
  return (
    <RecoveryScreen
      testId="relaunch-required"
      title="Restart Cortex Max to finish recovery"
      body="Your identity was updated. Cortex Max needs to restart so syncing and agents run under it."
    />
  );
}
