package hotdrop.agent;

import hotdrop.protocol.Wire;
import hotdrop.protocol.Wire.BeanChange;
import hotdrop.protocol.Wire.BeanResult;
import hotdrop.protocol.Wire.SpringChange;

import java.lang.instrument.ClassFileTransformer;
import java.lang.instrument.Instrumentation;
import java.lang.reflect.InvocationHandler;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.lang.reflect.Proxy;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Collections;
import java.util.HashSet;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.WeakHashMap;
import java.util.function.Consumer;

/**
 * Applies Spring changes to the running server. Everything goes through reflection: the agent must work with the
 * server's own Spring (5.3 and 6 differ in packages elsewhere, not in the handful of APIs used here) and cannot
 * link against it, because the agent sits on the system class path and Spring does not.
 * <p>
 * Contexts are found two ways: Hybris' {@code Registry} (polled), and a listener on every known context that
 * records child contexts when they finish refreshing (Spring publishes child events to the parent). Any other
 * application can call {@link SpringHook#register(Object)}.
 */
final class SpringBridge {
    private static final Set<Object> CONTEXTS = Collections.synchronizedSet(Collections.newSetFromMap(new WeakHashMap<>()));
    private static final Set<Object> LISTENING = Collections.synchronizedSet(Collections.newSetFromMap(new WeakHashMap<>()));

    private final Consumer<String> say;
    private volatile ClassLoader registryLoader;

    SpringBridge(Consumer<String> say) {
        this.say = say;
    }

    static void register(Object context) {
        if (context == null || !CONTEXTS.add(context)) return;
        try {
            ClassLoader l = context.getClass().getClassLoader();
            Class<?> listener = Class.forName("org.springframework.context.ApplicationListener", false, l);
            if (LISTENING.add(context)) {
                Object proxy = Proxy.newProxyInstance(l, new Class<?>[]{listener}, new Listener());
                call(context, "addApplicationListener", proxy);
            }
        } catch (Throwable t) {
            // not a Spring context after all, or Spring too old: still usable for XML changes
        }
    }

    boolean active() {
        return !CONTEXTS.isEmpty();
    }

    /** Child contexts announce themselves to listeners registered on their parent. */
    private static final class Listener implements InvocationHandler {
        @Override
        public Object invoke(Object proxy, Method m, Object[] args) {
            switch (m.getName()) {
                case "hashCode":
                    return System.identityHashCode(proxy);
                case "equals":
                    return proxy == args[0];
                case "toString":
                    return "HotDrop context listener";
                case "onApplicationEvent":
                    Object ev = args[0];
                    if (ev.getClass().getName().endsWith("ContextRefreshedEvent")) {
                        register(((java.util.EventObject) ev).getSource());
                    }
                    return null;
                default:
                    return null;
            }
        }
    }

    // ---- finding Hybris' contexts ----

    void startFinder(Instrumentation inst) {
        for (Class<?> c : inst.getAllLoadedClasses()) {
            if (c.getName().equals("de.hybris.platform.core.Registry")) registryLoader = c.getClassLoader();
        }
        if (registryLoader == null) watchForRegistry(inst);
        startPolling();
    }

    private void watchForRegistry(Instrumentation inst) {
        inst.addTransformer(new ClassFileTransformer() {
            @Override
            public byte[] transform(ClassLoader loader, String name, Class<?> redefined, java.security.ProtectionDomain pd, byte[] buf) {
                if ("de/hybris/platform/core/Registry".equals(name)) {
                    registryLoader = loader;
                    inst.removeTransformer(this);
                }
                return null;
            }
        });
    }

    private void startPolling() {
        Thread t = new Thread(() -> {
            long delay = 1500;
            while (true) {
                try {
                    Thread.sleep(delay);
                    if (findHybrisContexts()) delay = 30_000;
                } catch (InterruptedException e) {
                    return;
                } catch (Throwable ignored) {
                    // keep trying
                }
            }
        }, "hotdrop-spring-finder");
        t.setDaemon(true);
        t.start();
    }

    private boolean findHybrisContexts() {
        ClassLoader l = registryLoader;
        if (l == null) return false;
        boolean core = false;
        try {
            Class<?> registry = Class.forName("de.hybris.platform.core.Registry", false, l);
            for (String m : new String[]{"getGlobalApplicationContext", "getCoreApplicationContext"}) {
                try {
                    Object ctx = registry.getMethod(m).invoke(null);
                    if (ctx != null) {
                        register(ctx);
                        if (m.equals("getCoreApplicationContext")) core = true;
                    }
                } catch (Throwable ignored) {
                    // no tenant yet, or not started
                }
            }
        } catch (Throwable ignored) {
            // Registry not loadable from here
        }
        return core;
    }

    // ---- applying XML changes ----

    List<BeanResult> apply(SpringChange change) {
        String fileName = Path.of(change.file()).getFileName().toString();
        List<Object> owners = new ArrayList<>();
        synchronized (CONTEXTS) {
            for (Object ctx : CONTEXTS) {
                try {
                    if (owns(ctx, fileName, change.known())) owners.add(ctx);
                } catch (Throwable t) {
                    // closed or odd context
                }
            }
        }
        if (owners.isEmpty()) {
            String why = CONTEXTS.isEmpty() ? "no Spring context has been found in the server yet"
                    : CONTEXTS.size() + " context(s) known, none loaded " + fileName;
            return List.of(new BeanResult(fileName, Wire.BEAN_NO_CONTEXT, why));
        }
        Map<String, BeanResult> results = new LinkedHashMap<>();
        for (Object ctx : owners) {
            Thread cur = Thread.currentThread();
            ClassLoader saved = cur.getContextClassLoader();
            try {
                cur.setContextClassLoader((ClassLoader) call(ctx, "getClassLoader"));
                applyTo(ctx, change, results);
            } catch (Throwable t) {
                // typically the saved file does not parse (a class it names cannot be loaded yet): every bean in it fails
                List<String> ids = new ArrayList<>(change.added());
                for (BeanChange ch : change.changed()) ids.add(ch.id());
                for (String id : ids) results.putIfAbsent(id, new BeanResult(id, Wire.BEAN_FAILED, "the file could not be loaded: " + rootCause(t)));
            } finally {
                cur.setContextClassLoader(saved);
            }
        }
        return new ArrayList<>(results.values());
    }

    private boolean owns(Object ctx, String fileName, List<String> known) throws Exception {
        Object live = call(ctx, "getBeanFactory");
        for (String id : known) {
            if (!(Boolean) call(live, "containsBeanDefinition", id)) continue;
            Object desc = call(call(live, "getBeanDefinition", id), "getResourceDescription");
            if (desc != null && desc.toString().contains(fileName)) return true;
        }
        return false;
    }

    private void applyTo(Object ctx, SpringChange change, Map<String, BeanResult> results) throws Exception {
        Object live = call(ctx, "getBeanFactory");
        ClassLoader l = live.getClass().getClassLoader();
        Class<?> dlbf = Class.forName("org.springframework.beans.factory.support.DefaultListableBeanFactory", true, l);
        if (!dlbf.isInstance(live)) throw new IllegalStateException("unsupported bean factory " + live.getClass().getName());
        String where = String.valueOf(call(ctx, "getDisplayName"));

        // Parse the saved file with Spring itself, in a scratch factory, and resolve ${placeholders} the way the context does.
        Object scratch = dlbf.getConstructor().newInstance();
        ClassLoader beanLoader = (ClassLoader) call(live, "getBeanClassLoader");
        call(scratch, "setBeanClassLoader", beanLoader);
        Class<?> registryType = Class.forName("org.springframework.beans.factory.support.BeanDefinitionRegistry", true, l);
        Object reader = Class.forName("org.springframework.beans.factory.xml.XmlBeanDefinitionReader", true, l)
                .getConstructor(registryType).newInstance(scratch);
        call(reader, "setResourceLoader", ctx);
        call(reader, "setBeanClassLoader", beanLoader);
        call(reader, "setEnvironment", call(ctx, "getEnvironment"));
        Object resource = Class.forName("org.springframework.core.io.FileSystemResource", true, l)
                .getConstructor(String.class).newInstance(change.file());
        call(reader, "loadBeanDefinitions", resource);
        resolvePlaceholders(ctx, scratch, l);

        for (String id : change.added()) {
            results.put(id, add(live, scratch, id, where, l));
        }
        for (BeanChange ch : change.changed()) {
            if (!(Boolean) call(live, "containsBeanDefinition", ch.id())) continue;
            results.put(ch.id(), ch.props().isEmpty()
                    ? replaceCollection(live, scratch, ch.id(), where, dlbf)
                    : updateProperties(live, scratch, ch, where, l));
        }
    }

    private void resolvePlaceholders(Object ctx, Object scratch, ClassLoader l) {
        try {
            Class<?> support = Class.forName("org.springframework.beans.factory.config.PlaceholderConfigurerSupport", true, l);
            Map<?, ?> configurers = (Map<?, ?>) call(ctx, "getBeansOfType", support, Boolean.FALSE, Boolean.FALSE);
            for (Object cfg : configurers.values()) {
                try {
                    call(cfg, "postProcessBeanFactory", scratch);
                } catch (Throwable t) {
                    // an unresolved placeholder shows up when the bean is created
                }
            }
        } catch (Throwable t) {
            // no placeholder support in this context
        }
    }

    private BeanResult add(Object live, Object scratch, String id, String where, ClassLoader l) {
        try {
            if (!(Boolean) call(scratch, "containsBeanDefinition", id)) {
                return new BeanResult(id, Wire.BEAN_FAILED, "the saved file does not define it");
            }
            if ((Boolean) call(live, "containsBeanDefinition", id)) {
                return new BeanResult(id, Wire.BEAN_UPDATED, "already present in " + where);
            }
            Object def = call(scratch, "getBeanDefinition", id);
            call(live, "registerBeanDefinition", id, def);
            String[] aliases = (String[]) call(scratch, "getAliases", id);
            for (String alias : aliases) call(live, "registerAlias", id, alias);
            boolean eager = (Boolean) call(def, "isSingleton") && !(Boolean) call(def, "isLazyInit") && !(Boolean) call(def, "isAbstract");
            String note = "";
            if (eager) {
                try {
                    Object bean = call(live, "getBean", id);
                    if (isA(bean, "org.springframework.beans.factory.config.BeanPostProcessor", l)
                            || isA(bean, "org.springframework.beans.factory.config.BeanFactoryPostProcessor", l)) {
                        note = "; it is a post-processor, which only takes effect after a restart";
                    }
                } catch (Throwable t) {
                    try {
                        call(live, "removeBeanDefinition", id);
                        for (String alias : aliases) call(live, "removeAlias", alias);
                    } catch (Throwable ignored) {
                        // leave it
                    }
                    return new BeanResult(id, Wire.BEAN_FAILED, "creating it failed: " + rootCause(t));
                }
            }
            String msg = "context " + where + (eager ? "" : ", created when first used") + note;
            say.accept("Spring bean '" + id + "' added to context " + where);
            return new BeanResult(id, Wire.BEAN_ADDED, msg);
        } catch (Throwable t) {
            return new BeanResult(id, Wire.BEAN_FAILED, rootCause(t));
        }
    }

    private BeanResult updateProperties(Object live, Object scratch, BeanChange ch, String where, ClassLoader l) {
        String id = ch.id();
        try {
            if (!(Boolean) call(scratch, "containsBeanDefinition", id)) {
                return new BeanResult(id, Wire.BEAN_FAILED, "the saved file does not define it");
            }
            Object newDef = call(scratch, "getBeanDefinition", id);
            Object liveDef = call(live, "getBeanDefinition", id);
            Object liveValues = call(liveDef, "getPropertyValues");
            Object newValues = call(newDef, "getPropertyValues");
            Map<String, Object> pvs = new LinkedHashMap<>();
            for (String name : ch.props()) {
                Object pv = call(newValues, "getPropertyValue", name);
                if (pv == null) return new BeanResult(id, Wire.BEAN_FAILED, "property '" + name + "' is not in the saved file");
                pvs.put(name, pv);
            }
            boolean created = (Boolean) call(live, "containsSingleton", id);
            Object target = null;
            if (created) {
                Object bean = call(live, "getSingleton", id);
                if (isA(bean, "org.springframework.beans.factory.FactoryBean", l)) {
                    return new BeanResult(id, Wire.BEAN_FAILED, "it is a FactoryBean whose product is already built");
                }
                target = unwrap(bean, l);
            }
            // Instance first, then the definition: a failure on a later property leaves earlier ones set on the instance, and the
            // file stays pending so the next save retries.
            if (target != null) {
                Class<?> resolverType = Class.forName("org.springframework.beans.factory.support.BeanDefinitionValueResolver", true, l);
                java.lang.reflect.Constructor<?> rc = resolverType.getDeclaredConstructors()[0];
                rc.setAccessible(true);
                Object resolver = rc.newInstance(live, id, newDef, call(live, "getTypeConverter"));
                Object wrapper = Class.forName("org.springframework.beans.BeanWrapperImpl", true, l)
                        .getConstructor(Object.class).newInstance(target);
                call(live, "copyRegisteredEditorsTo", wrapper);
                Object conversion = call(live, "getConversionService");
                if (conversion != null) call(wrapper, "setConversionService", conversion);
                for (var e : pvs.entrySet()) {
                    Object value = call(resolver, "resolveValueIfNecessary", "bean property '" + e.getKey() + "'", call(e.getValue(), "getValue"));
                    call(wrapper, "setPropertyValue", e.getKey(), value);
                }
            }
            for (Object pv : pvs.values()) call(liveValues, "addPropertyValue", pv);
            call(live, "clearMetadataCache");
            String names = String.join(", ", ch.props());
            say.accept("Spring bean '" + id + "': " + names + " updated" + (target == null ? " (definition only, no instance yet)" : ""));
            return new BeanResult(id, Wire.BEAN_UPDATED, "set " + names + (target == null ? "; applies when the bean is created" : " on the live bean"));
        } catch (Throwable t) {
            return new BeanResult(id, Wire.BEAN_FAILED, rootCause(t));
        }
    }

    /** util:list / util:set / util:map: refill the live collection, so every bean already holding it sees the change. */
    private BeanResult replaceCollection(Object live, Object scratch, String id, String where, Class<?> dlbf) {
        try {
            if (!(Boolean) call(scratch, "containsBeanDefinition", id)) {
                return new BeanResult(id, Wire.BEAN_FAILED, "the saved file does not define it");
            }
            Object current = call(live, "getBean", id);
            Class<?> beanFactory = Class.forName("org.springframework.beans.factory.BeanFactory", true, dlbf.getClassLoader());
            Object child = dlbf.getConstructor(beanFactory).newInstance(live);
            call(child, "setBeanClassLoader", call(live, "getBeanClassLoader"));
            call(child, "registerBeanDefinition", id, call(scratch, "getBeanDefinition", id));
            Object fresh = call(child, "getBean", id);
            if (current instanceof List<?> && fresh instanceof Collection<?> c) {
                @SuppressWarnings("unchecked") List<Object> l = (List<Object>) current;
                List<Object> copy = new ArrayList<>(c);
                l.clear();
                l.addAll(copy);
            } else if (current instanceof Set<?> && fresh instanceof Collection<?> c) {
                @SuppressWarnings("unchecked") Set<Object> s = (Set<Object>) current;
                List<Object> copy = new ArrayList<>(c);
                s.clear();
                s.addAll(copy);
            } else if (current instanceof Map<?, ?> && fresh instanceof Map<?, ?> m) {
                @SuppressWarnings("unchecked") Map<Object, Object> t = (Map<Object, Object>) current;
                Map<Object, Object> copy = new LinkedHashMap<>(m);
                t.clear();
                t.putAll(copy);
            } else {
                return new BeanResult(id, Wire.BEAN_FAILED, "unsupported collection type " + current.getClass().getName());
            }
            call(live, "clearMetadataCache");
            say.accept("Spring collection '" + id + "' refilled in context " + where);
            return new BeanResult(id, Wire.BEAN_UPDATED, "collection replaced in place");
        } catch (Throwable t) {
            return new BeanResult(id, Wire.BEAN_FAILED, rootCause(t));
        }
    }

    // ---- after class swaps ----

    /** Spring caches reflection and annotation metadata per class; drop it for what was just redefined, and rebuild MVC mappings. */
    void afterSwap(List<Class<?>> swapped) {
        Set<ClassLoader> springLoaders = new LinkedHashSet<>();
        List<Object> contexts;
        synchronized (CONTEXTS) {
            contexts = new ArrayList<>(CONTEXTS);
        }
        for (Object ctx : contexts) springLoaders.add(ctx.getClass().getClassLoader());
        Set<ClassLoader> classLoaders = Collections.newSetFromMap(new IdentityHashMap<>());
        Set<String> names = new HashSet<>();
        for (Class<?> c : swapped) {
            if (c.getClassLoader() != null) classLoaders.add(c.getClassLoader());
            names.add(c.getName());
        }
        for (ClassLoader l : springLoaders) {
            for (String[] s : new String[][]{{"org.springframework.util.ReflectionUtils", "clearCache"},
                    {"org.springframework.core.annotation.AnnotationUtils", "clearCache"},
                    {"org.springframework.core.ResolvableType", "clearCache"}}) {
                try {
                    Class.forName(s[0], true, l).getMethod(s[1]).invoke(null);
                } catch (Throwable ignored) {
                    // class or method not in this Spring version
                }
            }
            try {
                Class<?> cir = Class.forName("org.springframework.beans.CachedIntrospectionResults", true, l);
                for (ClassLoader cl : classLoaders) cir.getMethod("clearClassLoader", ClassLoader.class).invoke(null, cl);
            } catch (Throwable ignored) {
                // not available
            }
        }
        for (Object ctx : contexts) {
            try {
                refreshMappings(ctx, names);
            } catch (Throwable t) {
                // not a web context
            }
        }
    }

    private void refreshMappings(Object ctx, Set<String> names) throws Exception {
        ClassLoader l = ctx.getClass().getClassLoader();
        Class<?> type;
        try {
            type = Class.forName("org.springframework.web.servlet.mvc.method.annotation.RequestMappingHandlerMapping", true, l);
        } catch (ClassNotFoundException e) {
            return;
        }
        Map<?, ?> mappings = (Map<?, ?>) call(ctx, "getBeansOfType", type, Boolean.FALSE, Boolean.FALSE);
        for (Object mapping : mappings.values()) {
            Map<?, ?> methods = (Map<?, ?>) call(mapping, "getHandlerMethods");
            List<Object> infos = new ArrayList<>();
            Set<Object> beans = new LinkedHashSet<>();
            for (var e : methods.entrySet()) {
                Class<?> beanType = (Class<?>) call(e.getValue(), "getBeanType");
                String n = beanType.getName();
                int cglib = n.indexOf("$$");
                if (names.contains(cglib < 0 ? n : n.substring(0, cglib))) {
                    infos.add(e.getKey());
                    beans.add(call(e.getValue(), "getBean"));
                }
            }
            if (infos.isEmpty()) continue;
            for (Object info : infos) call(mapping, "unregisterMapping", info);
            for (Object bean : beans) call(mapping, "detectHandlerMethods", bean);
            say.accept("Spring MVC mappings rebuilt for " + beans.size() + " handler bean(s)");
        }
    }

    // ---- reflection helpers ----

    private static boolean isA(Object o, String type, ClassLoader l) {
        try {
            return Class.forName(type, true, l).isInstance(o);
        } catch (ClassNotFoundException e) {
            return false;
        }
    }

    /** Beans wrapped by AOP are configured on the target, not on the proxy. */
    private static Object unwrap(Object bean, ClassLoader l) {
        try {
            Class<?> utils = Class.forName("org.springframework.aop.framework.AopProxyUtils", true, l);
            Object cur = bean;
            for (int i = 0; i < 10; i++) {
                Object next = utils.getMethod("getSingletonTarget", Object.class).invoke(null, cur);
                if (next == null) break;
                cur = next;
            }
            return cur;
        } catch (Throwable t) {
            return bean;
        }
    }

    static Object call(Object target, String name, Object... args) throws Exception {
        Method m = find(target.getClass(), name, args);
        try {
            return m.invoke(target, args);
        } catch (InvocationTargetException e) {
            Throwable c = e.getCause();
            if (c instanceof Exception ex) throw ex;
            if (c instanceof Error er) throw er;
            throw e;
        }
    }

    private static Method find(Class<?> type, String name, Object[] args) throws NoSuchMethodException {
        for (Class<?> c = type; c != null; c = c.getSuperclass()) {
            List<Method> candidates = new ArrayList<>(List.of(c.getDeclaredMethods()));
            for (Class<?> i : c.getInterfaces()) candidates.addAll(List.of(i.getMethods()));
            for (Method m : candidates) {
                if (!m.getName().equals(name) || m.getParameterCount() != args.length || Modifier.isStatic(m.getModifiers())) continue;
                if (!accepts(m.getParameterTypes(), args)) continue;
                try {
                    m.setAccessible(true);
                } catch (RuntimeException ignored) {
                    // stay with what is public
                }
                return m;
            }
        }
        throw new NoSuchMethodException(type.getName() + "." + name + " with " + args.length + " argument(s)");
    }

    private static boolean accepts(Class<?>[] params, Object[] args) {
        for (int i = 0; i < params.length; i++) {
            Class<?> p = params[i];
            if (args[i] == null) {
                if (p.isPrimitive()) return false;
                continue;
            }
            if (p.isPrimitive()) {
                if (p == boolean.class && args[i] instanceof Boolean) continue;
                if (p == int.class && args[i] instanceof Integer) continue;
                if (p == long.class && args[i] instanceof Long) continue;
                return false;
            }
            if (!p.isInstance(args[i])) return false;
        }
        return true;
    }

    private static String rootCause(Throwable t) {
        Throwable c = t;
        for (int i = 0; i < 20 && c.getCause() != null && c.getCause() != c; i++) c = c.getCause();
        String m = c.getMessage();
        return c.getClass().getSimpleName() + (m == null ? "" : ": " + (m.length() > 300 ? m.substring(0, 300) + "..." : m));
    }
}
