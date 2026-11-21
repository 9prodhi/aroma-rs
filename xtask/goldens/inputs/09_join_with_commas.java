import java.util.List;

class JoinWithCommas {
    String join(List<String> items) {
        StringBuilder sb = new StringBuilder();
        int index = 0;
        for (String item : items) {
            if (index > 0) {
                sb.append(", ");
            }
            sb.append(item);
            index++;
        }
        sb.append(".");
        String result = sb.toString();
        return result;
    }
}
