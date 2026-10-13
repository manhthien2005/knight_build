/**
 * State enum for Boss Hunt finite state machine.
 * Defines execution phases and control ownership flags.
 */
public enum BossHuntState {
    IDLE(false),
    NEED_SCHEDULE(false),
    WAIT_DEPART(false),
    TRAVEL(true),
    SWITCH_ZONE(true),
    PATROL(true),
    COMBAT(true),
    RECOVER(false),
    PAUSED(false),
    BLOCKED(false);

    private final boolean controlsMovement;

    BossHuntState(boolean controlsMovement) {
        this.controlsMovement = controlsMovement;
    }

    public boolean controlsMovement() {
        return controlsMovement;
    }
}
