package hotdrop.daemon;

import java.time.LocalTime;
import java.time.format.DateTimeFormatter;

final class Log {
    private static final DateTimeFormatter TS = DateTimeFormatter.ofPattern("HH:mm:ss.SSS");
    static volatile boolean debug;

    private Log() {}

    static void info(String fmt, Object... args) {
        System.out.println(LocalTime.now().format(TS) + " " + String.format(fmt, args));
    }

    static void warn(String fmt, Object... args) {
        System.out.println(LocalTime.now().format(TS) + " ! " + String.format(fmt, args));
    }

    static void debug(String fmt, Object... args) {
        if (debug) System.out.println(LocalTime.now().format(TS) + " . " + String.format(fmt, args));
    }

    static String ms(long nanos) {
        return (nanos / 1_000_000) + "ms";
    }
}
