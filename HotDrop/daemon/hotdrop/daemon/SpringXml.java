package hotdrop.daemon;

import hotdrop.protocol.Wire.BeanChange;
import hotdrop.protocol.Wire.SpringChange;
import org.w3c.dom.Element;
import org.w3c.dom.NamedNodeMap;
import org.w3c.dom.Node;

import javax.xml.parsers.DocumentBuilderFactory;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;

/**
 * Compares two versions of a Spring XML file at the DOM level, without Spring: which top-level beans were added,
 * which only had {@code <property>} values changed, and which changes cannot be applied to a running context.
 * Elements are compared in a canonical form (sorted attributes, no comments, trimmed text), so reformatting a file
 * changes nothing.
 */
final class SpringXml {
    private static final String BEANS_NS = "http://www.springframework.org/schema/beans";
    private static final String UTIL_NS = "http://www.springframework.org/schema/util";

    /** A parsed top-level element that has an id. */
    private record Bean(String id, String whole, String shell, Map<String, String> props, boolean collection) {}

    /** The result of comparing two versions. {@code restart} lists what could not be applied, with the reason. */
    record Diff(SpringChange change, List<String> restart) {
        boolean empty() {
            return change.added().isEmpty() && change.changed().isEmpty() && restart.isEmpty();
        }
    }

    /** What a file looked like when last seen. */
    static final class Snapshot {
        final Map<String, Bean> beans;
        /** Canonical form of every top-level element that is not a bean with an id (alias, import, context:, ...). */
        final Set<String> others;

        Snapshot(Map<String, Bean> beans, Set<String> others) {
            this.beans = beans;
            this.others = others;
        }
    }

    private SpringXml() {}

    /** True for the file names Hybris uses for Spring configuration. */
    static boolean isSpringFile(Path p) {
        String n = p.getFileName() == null ? "" : p.getFileName().toString();
        return n.endsWith("-spring.xml") || n.endsWith("-servlet.xml");
    }

    static Snapshot read(Path file) throws Exception {
        DocumentBuilderFactory dbf = DocumentBuilderFactory.newInstance();
        dbf.setNamespaceAware(true);
        // Older Spring files carry a DOCTYPE; accept it but never fetch anything it points to.
        dbf.setFeature("http://xml.org/sax/features/external-general-entities", false);
        dbf.setFeature("http://xml.org/sax/features/external-parameter-entities", false);
        dbf.setFeature("http://apache.org/xml/features/nonvalidating/load-external-dtd", false);
        Element root;
        try (var in = Files.newInputStream(file)) {
            var builder = dbf.newDocumentBuilder();
            builder.setEntityResolver((publicId, systemId) -> new org.xml.sax.InputSource(new java.io.StringReader("")));
            root = builder.parse(in).getDocumentElement();
        }
        Map<String, Bean> beans = new LinkedHashMap<>();
        Set<String> others = new LinkedHashSet<>();
        for (Node n = root.getFirstChild(); n != null; n = n.getNextSibling()) {
            if (!(n instanceof Element e)) continue;
            String id = e.getAttribute("id");
            boolean bean = isBeansElement(e, "bean");
            boolean collection = UTIL_NS.equals(e.getNamespaceURI())
                    && (e.getLocalName().equals("list") || e.getLocalName().equals("set") || e.getLocalName().equals("map"));
            if (id.isEmpty() && bean) id = firstName(e.getAttribute("name"));
            if (id.isEmpty() || !(bean || collection)) {
                others.add(canonical(e));
                continue;
            }
            Map<String, String> props = new TreeMap<>();
            String shell;
            if (bean) {
                StringBuilder sb = new StringBuilder(attrs(e));
                for (Node c = e.getFirstChild(); c != null; c = c.getNextSibling()) {
                    if (!(c instanceof Element ce)) continue;
                    if (isBeansElement(ce, "property")) {
                        props.put(ce.getAttribute("name"), canonical(ce));
                    } else {
                        sb.append(canonical(ce));
                    }
                }
                shell = sb.toString();
            } else {
                shell = "collection";
            }
            beans.put(id, new Bean(id, canonical(e), shell, props, collection));
        }
        return new Snapshot(beans, others);
    }

    /** DTD-style files (with a DOCTYPE) have no namespace on their elements. */
    private static boolean isBeansElement(Element e, String name) {
        String ns = e.getNamespaceURI();
        return (ns == null || BEANS_NS.equals(ns)) && name.equals(e.getLocalName());
    }

    private static String firstName(String names) {
        for (String n : names.split("[,; ]")) if (!n.isEmpty()) return n;
        return "";
    }

    static Diff diff(Path file, Snapshot old, Snapshot now) {
        List<String> added = new ArrayList<>();
        List<BeanChange> changed = new ArrayList<>();
        List<String> restart = new ArrayList<>();
        for (Bean b : now.beans.values()) {
            Bean was = old.beans.get(b.id());
            if (was == null) {
                added.add(b.id());
            } else if (!was.whole().equals(b.whole())) {
                if (b.collection() && was.collection()) {
                    changed.add(new BeanChange(b.id(), List.of()));
                } else if (b.collection() != was.collection() || !was.shell().equals(b.shell())) {
                    restart.add("bean '" + b.id() + "': class, scope, constructor or other settings changed");
                } else {
                    List<String> props = new ArrayList<>();
                    for (var p : b.props().entrySet()) {
                        if (!p.getValue().equals(was.props().get(p.getKey()))) props.add(p.getKey());
                    }
                    for (String p : was.props().keySet()) {
                        if (!b.props().containsKey(p)) restart.add("bean '" + b.id() + "': property '" + p + "' removed");
                    }
                    if (!props.isEmpty()) changed.add(new BeanChange(b.id(), props));
                }
            }
        }
        for (String id : old.beans.keySet()) {
            if (!now.beans.containsKey(id)) restart.add("bean '" + id + "' removed");
        }
        if (!old.others.equals(now.others)) {
            restart.add("an alias, import or namespace element (context:, aop:, ...) changed");
        }
        SpringChange c = new SpringChange(file.toString(), added, changed, new ArrayList<>(old.beans.keySet()));
        return new Diff(c, restart);
    }

    /** The baseline after a partial apply: {@code old}, with the beans in {@code settled} taken from {@code now}. */
    static Snapshot advance(Snapshot old, Snapshot now, Set<String> settled) {
        Map<String, Bean> beans = new LinkedHashMap<>(old.beans);
        for (String id : settled) {
            Bean b = now.beans.get(id);
            if (b != null) beans.put(id, b);
        }
        return new Snapshot(beans, old.others);
    }

    // ---- canonical form ----

    private static String attrs(Element e) {
        NamedNodeMap m = e.getAttributes();
        TreeMap<String, String> sorted = new TreeMap<>();
        for (int i = 0; i < m.getLength(); i++) {
            Node a = m.item(i);
            sorted.put(a.getNodeName(), a.getNodeValue().strip());
        }
        String ns = e.getNamespaceURI() == null ? BEANS_NS : e.getNamespaceURI();
        StringBuilder sb = new StringBuilder("<").append(ns).append('|').append(e.getLocalName());
        sorted.forEach((k, v) -> sb.append(' ').append(k).append('=').append(v));
        return sb.append('>').toString();
    }

    private static String canonical(Element e) {
        StringBuilder sb = new StringBuilder(attrs(e));
        for (Node c = e.getFirstChild(); c != null; c = c.getNextSibling()) {
            if (c instanceof Element ce) {
                sb.append(canonical(ce));
            } else if (c.getNodeType() == Node.TEXT_NODE || c.getNodeType() == Node.CDATA_SECTION_NODE) {
                String t = c.getNodeValue().strip();
                if (!t.isEmpty()) sb.append('"').append(t).append('"');
            }
        }
        return sb.append("</>").toString();
    }

    /** Reads a file, returning null (and logging) when it is unreadable or not well-formed yet. */
    static Snapshot tryRead(Path file) {
        try {
            return read(file);
        } catch (IOException e) {
            return null;
        } catch (Exception e) {
            Log.warn("%s is not well-formed XML yet (%s); will retry on the next save", file.getFileName(), firstLine(e));
            return null;
        }
    }

    private static String firstLine(Exception e) {
        String m = String.valueOf(e.getMessage());
        int nl = m.indexOf('\n');
        return nl < 0 ? m : m.substring(0, nl);
    }
}
