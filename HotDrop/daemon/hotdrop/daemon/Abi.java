package hotdrop.daemon;

import javax.lang.model.element.Element;
import javax.lang.model.element.ExecutableElement;
import javax.lang.model.element.Modifier;
import javax.lang.model.element.TypeElement;
import javax.lang.model.element.VariableElement;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

/** A hash of everything other classes can see of a type: non-private members, signatures, constant values. */
final class Abi {
    private Abi() {}

    static String hash(TypeElement type) {
        StringBuilder sb = new StringBuilder();
        append(type, sb);
        return sha1(sb.toString().getBytes(StandardCharsets.UTF_8));
    }

    private static void append(TypeElement t, StringBuilder sb) {
        sb.append(t.getKind()).append(' ').append(mods(t)).append(' ').append(t.getQualifiedName());
        t.getTypeParameters().forEach(tp -> sb.append(" <").append(tp).append(" extends ").append(tp.getBounds()).append('>'));
        sb.append(" extends ").append(t.getSuperclass()).append(" implements ").append(t.getInterfaces());
        List<String> members = new ArrayList<>();
        for (Element m : t.getEnclosedElements()) {
            if (m.getModifiers().contains(Modifier.PRIVATE)) continue;
            StringBuilder s = new StringBuilder();
            if (m instanceof TypeElement nested) {
                s.append("T:");
                append(nested, s);
            } else {
                s.append(m.getKind()).append(' ').append(mods(m)).append(' ').append(m.getSimpleName())
                        .append(' ').append(m.asType());
                if (m instanceof VariableElement v && v.getConstantValue() != null) {
                    s.append(" = ").append(v.getConstantValue());
                }
                if (m instanceof ExecutableElement ex) {
                    s.append(" throws ").append(ex.getThrownTypes());
                    if (ex.getDefaultValue() != null) s.append(" default ").append(ex.getDefaultValue());
                }
            }
            members.add(s.toString());
        }
        Collections.sort(members);
        for (String m : members) sb.append('\n').append(m);
    }

    private static String mods(Element e) {
        List<String> l = new ArrayList<>();
        e.getModifiers().forEach(m -> l.add(m.toString()));
        Collections.sort(l);
        return String.join(",", l);
    }

    static String sha1(byte[] b) {
        try {
            byte[] d = MessageDigest.getInstance("SHA-1").digest(b);
            StringBuilder sb = new StringBuilder();
            for (byte x : d) sb.append(String.format("%02x", x));
            return sb.toString();
        } catch (java.security.NoSuchAlgorithmException e) {
            throw new IllegalStateException(e);
        }
    }
}
