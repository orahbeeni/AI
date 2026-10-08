package hotdrop.daemon;

import com.sun.source.tree.IdentifierTree;
import com.sun.source.tree.MemberReferenceTree;
import com.sun.source.tree.MemberSelectTree;
import com.sun.source.tree.NewClassTree;
import com.sun.source.util.TreePath;
import com.sun.source.util.TreePathScanner;
import com.sun.source.util.Trees;

import javax.lang.model.element.Element;
import javax.lang.model.element.ElementKind;
import javax.lang.model.element.TypeElement;
import java.util.Set;

/**
 * Collects the top-level types a class body refers to. A reference to a member counts as a reference to the
 * type that declares it, so {@code a.getB().doIt()} records both A and B even though B is never named.
 */
final class RefScanner extends TreePathScanner<Void, Set<String>> {
    private final Trees trees;

    RefScanner(Trees trees) {
        this.trees = trees;
    }

    @Override
    public Void visitIdentifier(IdentifierTree node, Set<String> out) {
        record(getCurrentPath(), out);
        return super.visitIdentifier(node, out);
    }

    @Override
    public Void visitMemberSelect(MemberSelectTree node, Set<String> out) {
        record(getCurrentPath(), out);
        return super.visitMemberSelect(node, out);
    }

    @Override
    public Void visitNewClass(NewClassTree node, Set<String> out) {
        record(getCurrentPath(), out);
        return super.visitNewClass(node, out);
    }

    @Override
    public Void visitMemberReference(MemberReferenceTree node, Set<String> out) {
        record(getCurrentPath(), out);
        return super.visitMemberReference(node, out);
    }

    private void record(TreePath path, Set<String> out) {
        try {
            Element e = trees.getElement(path);
            if (e == null) return;
            TypeElement top = null;
            for (Element x = e; x != null && x.getKind() != ElementKind.PACKAGE; x = x.getEnclosingElement()) {
                if (x instanceof TypeElement te) top = te;
            }
            if (top != null) out.add(top.getQualifiedName().toString());
        } catch (RuntimeException ignored) {
            // an unresolved symbol in a broken file: nothing to record
        }
    }
}
