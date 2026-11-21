import java.util.Iterator;
import java.util.List;

class RetainEven {
    int retainEven(List<Integer> values) {
        int removed = 0;
        Iterator<Integer> it = values.iterator();
        while (it.hasNext()) {
            Integer value = it.next();
            if (value % 2 != 0) {
                it.remove();
                removed++;
            }
        }
        int remaining = values.size();
        System.out.println(remaining + " kept");
        return removed;
    }
}
