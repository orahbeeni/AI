package hotdrop.agent;

/**
 * For applications that are not SAP Commerce (and for tests): tell HotDrop about an ApplicationContext so Spring XML
 * and class changes are applied to it. Hybris' contexts are found automatically.
 */
public final class SpringHook {
    private SpringHook() {}

    public static void register(Object applicationContext) {
        SpringBridge.register(applicationContext);
    }
}
