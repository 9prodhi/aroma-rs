import java.util.Iterator;
import java.util.List;

class RemoveEmptyStrings {
    int removeEmpty(List<String> values) {
        int removed = 0;
        Iterator<String> it = values.iterator();
        while (it.hasNext()) {
            String value = it.next();
            if (value == null || value.isEmpty()) {
                it.remove();
                removed++;
            }
        }
        int remaining = values.size();
        System.out.println(remaining);
        return removed;
    }
}
