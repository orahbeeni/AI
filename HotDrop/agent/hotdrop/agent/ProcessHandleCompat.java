package hotdrop.agent;

final class ProcessHandleCompat {
    private ProcessHandleCompat() {}

    static long pid() {
        return ProcessHandle.current().pid();
    }
}
