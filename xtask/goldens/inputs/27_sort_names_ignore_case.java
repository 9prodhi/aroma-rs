import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

class SortNamesIgnoreCase {
    List<String> sortNames(List<String> names) {
        List<String> sorted = new ArrayList<>(names);
        Collections.sort(sorted, String.CASE_INSENSITIVE_ORDER);
        int size = sorted.size();
        if (size > 1 && sorted.get(0).isEmpty()) {
            sorted.remove(0);
        }
        System.out.println(size);
        return sorted;
    }
}
