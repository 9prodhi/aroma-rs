import java.util.List;

class JoinReversed {
    String joinReversed(List<String> items) {
        StringBuilder sb = new StringBuilder();
        int index = items.size() - 1;
        while (index >= 0) {
            sb.append(items.get(index));
            if (index > 0) {
                sb.append(", ");
            }
            index--;
        }
        sb.append(".");
        String result = sb.toString();
        return result;
    }
}
